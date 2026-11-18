//! Moving one stored secret to the name a pulled rename gave it.

use super::SecretName;
use crate::collection::Collection;
use crate::error::{EnvError, RenameRefusal};
use crate::secrets::{SecretError, SecretKey, SecretStore};

impl Collection {
    /// Move the value stored under `from` to `to`, then remove `from`. Refused, with nothing
    /// changed, unless `from` is orphaned now, `to` is declared secret, a value is stored under
    /// `from`, and `to` is free or holds the same value. [`EnvError::RenameNotMoved`] when a
    /// store call fails or the written value does not read back; the old key is then kept.
    pub fn move_renamed_secret(
        &self,
        from: &SecretName,
        to: &SecretName,
        store: &dyn SecretStore,
    ) -> Result<(), EnvError> {
        let mv = Move::new(self, from, to, store);
        mv.check()?;
        let Some(value) = mv.get(&mv.from_key)? else {
            return Err(mv.refused(RenameRefusal::NothingStored));
        };
        let written = match mv.get(&mv.to_key)? {
            None => {
                if let Err(error) = store.set(&mv.to_key, &value) {
                    mv.undo_write();
                    return Err(mv.not_moved(error));
                }
                true
            }
            Some(there) if there == value => false,
            Some(_) => return Err(mv.refused(RenameRefusal::Taken)),
        };
        // The old key goes only once the new one reads back the same value; anything else
        // removes what this call wrote and keeps the original.
        let back = match store.get(&mv.to_key) {
            Ok(back) => back,
            Err(error) => {
                if written {
                    mv.undo_write();
                }
                return Err(mv.not_moved(error));
            }
        };
        if back.as_deref() != Some(value.as_str()) {
            if written {
                mv.undo_write();
            }
            return Err(mv.not_moved(SecretError::Failed {
                message: "the value read back differs from the one written".to_owned(),
            }));
        }
        store
            .delete(&mv.from_key)
            .map_err(|error| mv.not_moved(error))
    }

    /// Remove the orphaned key `from` once a value has been typed in under `to`. `Ok(false)`,
    /// with nothing removed, while `to` holds nothing. Refused unless `from` is orphaned now and
    /// `to` is declared secret, so a value still in use is never removed.
    pub fn finish_reentered(
        &self,
        from: &SecretName,
        to: &SecretName,
        store: &dyn SecretStore,
    ) -> Result<bool, EnvError> {
        let mv = Move::new(self, from, to, store);
        mv.check()?;
        if mv.get(&mv.to_key)?.is_none() {
            return Ok(false);
        }
        store
            .delete(&mv.from_key)
            .map_err(|error| mv.not_moved(error))?;
        Ok(true)
    }
}

/// One move's names, keys and store, so each step reports against the same pair.
struct Move<'a> {
    collection: &'a Collection,
    from: &'a SecretName,
    to: &'a SecretName,
    from_key: SecretKey,
    to_key: SecretKey,
    store: &'a dyn SecretStore,
}

impl<'a> Move<'a> {
    fn new(
        collection: &'a Collection,
        from: &'a SecretName,
        to: &'a SecretName,
        store: &'a dyn SecretStore,
    ) -> Move<'a> {
        Move {
            collection,
            from,
            to,
            from_key: collection.secret_key(&from.environment, &from.variable),
            to_key: collection.secret_key(&to.environment, &to.variable),
            store,
        }
    }

    /// Re-derive from the files and store now that `from` is orphaned and `to` is declared
    /// secret, whatever the scan that offered the move saw.
    fn check(&self) -> Result<(), EnvError> {
        let report = match self.collection.all_orphaned_secrets(self.store) {
            Ok(report) => report,
            Err(EnvError::SecretListRoot { error, .. }) => return Err(self.not_moved(error)),
            Err(e) => return Err(e),
        };
        let orphaned = report
            .orphans
            .iter()
            .any(|o| o.environment == self.from.environment && o.variable == self.from.variable);
        if !orphaned {
            return Err(self.refused(RenameRefusal::NotOrphaned));
        }
        let declared = self.collection.declared_on_disk(&self.to.environment);
        if !declared.contains(&self.to.variable) {
            return Err(self.refused(RenameRefusal::NotDeclared));
        }
        Ok(())
    }

    fn get(&self, key: &SecretKey) -> Result<Option<String>, EnvError> {
        self.store.get(key).map_err(|error| self.not_moved(error))
    }

    /// Best effort: the caller already reports the failure that led here.
    fn undo_write(&self) {
        let _ = self.store.delete(&self.to_key);
    }

    fn refused(&self, reason: RenameRefusal) -> EnvError {
        EnvError::RenameRefused {
            from: self.from.clone(),
            to: self.to.clone(),
            reason,
        }
    }

    fn not_moved(&self, error: SecretError) -> EnvError {
        EnvError::RenameNotMoved {
            from: self.from.clone(),
            to: self.to.clone(),
            error: Box::new(error),
        }
    }
}
