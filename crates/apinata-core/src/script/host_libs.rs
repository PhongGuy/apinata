//! `uuid`, `nanoid` and `crypto` backed by the host's generators and hash functions.

use hmac::{Hmac, KeyInit, Mac};
use md5::Md5;
use rand::TryRngCore;
use rand::rngs::OsRng;
use rquickjs::function::Opt;
use rquickjs::{Ctx, Exception, Function};
use sha1::{Digest, Sha1};
use sha2::{Sha224, Sha256, Sha384, Sha512};
use uuid::Uuid;

use super::engine::Installer;

const HOST_LIBS_JS: &str = include_str!("host_libs.js");

/// The most random bytes one call may ask for, as the Web Crypto quota.
const MAX_RANDOM_BYTES: usize = 65_536;

/// Installs `__uuid`, `__random`, `__hash` and `host_libs.js`, which builds the globals over them.
pub(crate) fn installer() -> Installer {
    Box::new(|ctx| {
        ctx.globals()
            .set("__uuid", Function::new(ctx.clone(), generate)?)?;
        ctx.globals()
            .set("__random", Function::new(ctx.clone(), random)?)?;
        ctx.globals()
            .set("__hash", Function::new(ctx.clone(), hash)?)?;
        ctx.globals()
            .set("__proxyTarget", Function::new(ctx.clone(), super::libs::proxy_target)?)?;
        ctx.eval::<(), _>(HOST_LIBS_JS)
    })
}

/// One uuid for `op` (`v1` to `v7`); `args` is a JSON array, `[name, namespace]` for v3 and v5.
fn generate<'js>(ctx: Ctx<'js>, op: String, args: String) -> rquickjs::Result<String> {
    let invalid = || Exception::throw_type(&ctx, "Invalid UUID");
    let named = || -> Option<(String, Uuid)> {
        let [name, namespace]: [String; 2] = serde_json::from_str(&args).ok()?;
        Some((name, Uuid::parse_str(&namespace).ok()?))
    };
    let mut node = [0u8; 6];
    fill(&ctx, &mut node)?;
    let id = match op.as_str() {
        "v1" => Uuid::now_v1(&node),
        "v3" => {
            let (name, namespace) = named().ok_or_else(invalid)?;
            Uuid::new_v3(&namespace, name.as_bytes())
        }
        "v4" => Uuid::new_v4(),
        "v5" => {
            let (name, namespace) = named().ok_or_else(invalid)?;
            Uuid::new_v5(&namespace, name.as_bytes())
        }
        "v6" => Uuid::now_v6(&node),
        "v7" => Uuid::now_v7(),
        _ => return Err(invalid()),
    };
    Ok(id.hyphenated().to_string())
}

/// `count` random bytes as a JSON array; a count that is not a whole number up to the quota throws.
fn random<'js>(ctx: Ctx<'js>, count: f64) -> rquickjs::Result<String> {
    if !(count >= 0.0 && count.fract() == 0.0 && count <= MAX_RANDOM_BYTES as f64) {
        return Err(Exception::throw_range(
            &ctx,
            "the requested length exceeds 65536 bytes",
        ));
    }
    let mut bytes = vec![0u8; count as usize];
    fill(&ctx, &mut bytes)?;
    Ok(serde_json::to_string(&bytes).unwrap_or_else(|_| "[]".into()))
}

/// The raw digest of `data` under `algorithm`, or its HMAC when a `key` is given. The shim checks
/// the name first; an unknown one still throws here.
fn hash<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    data: Vec<u8>,
    key: Opt<Vec<u8>>,
) -> rquickjs::Result<Vec<u8>> {
    macro_rules! digest {
        ($d:ty) => {
            match &key.0 {
                None => <$d>::digest(&data).to_vec(),
                Some(key) => {
                    let mut mac = <Hmac<$d> as KeyInit>::new_from_slice(key)
                        .expect("HMAC takes any key length");
                    mac.update(&data);
                    mac.finalize().into_bytes().to_vec()
                }
            }
        };
    }
    Ok(match algorithm.as_str() {
        "md5" => digest!(Md5),
        "sha1" => digest!(Sha1),
        "sha224" => digest!(Sha224),
        "sha256" => digest!(Sha256),
        "sha384" => digest!(Sha384),
        "sha512" => digest!(Sha512),
        _ => {
            return Err(Exception::throw_message(
                &ctx,
                &format!(
                    "Digest method not supported: \"{algorithm}\" (supported: md5, sha1, sha224, sha256, sha384, sha512; require('crypto-js') has more)"
                ),
            ));
        }
    })
}

/// Fills `bytes` from the operating system; a failing source throws instead of returning zeros.
fn fill(ctx: &Ctx<'_>, bytes: &mut [u8]) -> rquickjs::Result<()> {
    OsRng
        .try_fill_bytes(bytes)
        .map_err(|_| Exception::throw_message(ctx, "no secure random source is available"))
}
