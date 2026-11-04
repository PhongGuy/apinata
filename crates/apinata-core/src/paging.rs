//! Paging rules shared by the item listing and the search.

/// The most items or hits one page holds; a larger `limit` is clamped to it.
pub const MAX_PAGE_LIMIT: usize = 500;

/// Why a page request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PageError {
    /// A page of no items can never make progress.
    #[error("limit must be at least 1")]
    ZeroLimit,
    /// Depth counts the levels to list, so 0 would list nothing.
    #[error("depth must be at least 1")]
    ZeroDepth,
}

/// The limit a page actually uses: refuses 0 and clamps to [`MAX_PAGE_LIMIT`].
pub(crate) fn effective_limit(limit: Option<usize>) -> Result<Option<usize>, PageError> {
    match limit {
        Some(0) => Err(PageError::ZeroLimit),
        other => Ok(other.map(|n| n.min(MAX_PAGE_LIMIT))),
    }
}
