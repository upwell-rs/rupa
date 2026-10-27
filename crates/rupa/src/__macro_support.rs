//! Paths used by `rupa-macros` output. Not public API.

pub use ::std::boxed::Box;
pub use ::std::future::Future;
pub use ::std::marker::Send;
pub use ::std::pin::Pin;
pub use rupa_core::__macro_support::*;
pub use rupa_core::exec::Executor;
#[cfg(feature = "async")]
pub use rupa_core::exec::{AsyncExecutor, complete_unboxed};
pub use rupa_core::repo::{
    Acquire, Context, ContextMut, DynRepository, ExclusiveSyncMode, Repo, RepoMut, RepoView,
    Repository, SyncMode,
};
#[cfg(feature = "async")]
pub use rupa_core::repo::{AcquireAsync, AsyncMode, ExclusiveAsyncMode};

/// Runs `future` to completion from a sync repository method of an async
/// repository.
#[cfg(feature = "async")]
pub fn block_in_place<F: Future>(future: F) -> F::Output {
    #[cfg(feature = "tokio")]
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
    {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }

    pollster::block_on(future)
}
