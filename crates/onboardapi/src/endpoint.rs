//! Generic service/client endpoint over the C++ shim.

use std::{
    ffi::c_void,
    fmt,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    sync::Arc,
};

use tokio::sync::mpsc;
use tracing::{trace, warn};

use crate::{
    Error, Result,
    error::check,
    wire::{Reader, WireError, Writer},
};

/// One interface's operation set (`ServiceMessage` / `ClientMessage` of a module).
pub trait Message: Sized + Send + 'static {
    /// IDL interface, e.g. `M_CoordinateFrame::IService`.
    const INTERFACE: &'static str;

    fn op_id(&self) -> u32;
    fn op_name(&self) -> &'static str;
    fn encode_args(&self, w: &mut Writer);
    fn decode(op: u32, r: &mut Reader<'_>) -> Result<Self, WireError>;
}

/// One side (service or client) of an interface module.
pub trait Role: 'static {
    /// Operations this side receives.
    type Incoming: Message;
    /// Operations this side sends.
    type Outgoing: Message;
    /// e.g. `M_CoordinateFrame::Service`
    const NAME: &'static str;

    fn vtable() -> &'static onboardapi_sys::obapi_vtable;
}

/// Marks client roles, which can wait for their service and carry a display name.
pub trait ClientSide: Role {}

/// Async dispatch of a message to its handler trait (`ServiceHandler` / `ClientHandler`).
#[async_trait::async_trait]
pub trait Dispatch<H: ?Sized + Send>: Message {
    async fn dispatch(self, handler: &mut H) -> anyhow::Result<()>;
}

/// Context handed to the C++ callbacks.
struct Ctx<M> {
    tx: mpsc::UnboundedSender<M>,
}

struct Inner<R: Role> {
    handle: NonNull<c_void>,
    ctx: Option<NonNull<Ctx<R::Incoming>>>,
    domain: i32,
    service_name: String,
}

// SAFETY: the shim serializes `send`/`set_name` per endpoint with a mutex, `is_valid`
// is a read-only query, and `destroy` only runs once from `Drop` when no other
// reference exists. The callback context is only touched by the shim's dispatcher.
unsafe impl<R: Role> Send for Inner<R> {}
unsafe impl<R: Role> Sync for Inner<R> {}

impl<R: Role> Drop for Inner<R> {
    fn drop(&mut self) {
        // SAFETY: handle came from `create`; destroy blocks until in-flight callbacks
        // return and guarantees no callback runs afterwards, so freeing ctx is safe.
        unsafe { (R::vtable().destroy)(self.handle.as_ptr()) };
        if let Some(ctx) = self.ctx.take() {
            // SAFETY: created by Box::into_raw in `create`, no longer referenced.
            drop(unsafe { Box::from_raw(ctx.as_ptr()) });
        }
    }
}

/// A service or client of one onboardapi interface module.
///
/// Use the module aliases, e.g. [`crate::coordinate_frame::Service`] or
/// [`crate::coordinate_frame::Client`]. Cloning is cheap; the DDS endpoint is removed
/// when the last clone is dropped (late-join data is then marked `IsRemoved`).
pub struct Endpoint<R: Role> {
    inner: Arc<Inner<R>>,
    _role: PhantomData<fn() -> R>,
}

impl<R: Role> Clone for Endpoint<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _role: PhantomData,
        }
    }
}

impl<R: Role> fmt::Debug for Endpoint<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(R::NAME)
            .field("domain", &self.inner.domain)
            .field("service_name", &self.inner.service_name)
            .field("valid", &self.is_valid())
            .finish()
    }
}

/// Incoming operations of an endpoint, in arrival order.
///
/// Messages are queued without bound; drain the inbox continuously, especially for
/// high-rate `Stream*` operations.
#[derive(Debug)]
pub struct Inbox<M> {
    rx: mpsc::UnboundedReceiver<M>,
}

impl<M: Message> Inbox<M> {
    /// Next message; `None` once the endpoint is dropped.
    pub async fn recv(&mut self) -> Option<M> {
        self.rx.recv().await
    }

    /// Next message without waiting.
    pub fn try_recv(&mut self) -> Option<M> {
        self.rx.try_recv().ok()
    }

    /// Blocking receive for non-async code. Panics inside a tokio runtime.
    pub fn blocking_recv(&mut self) -> Option<M> {
        self.rx.blocking_recv()
    }

    /// Feeds every message to `handler` until the endpoint is dropped or the handler fails.
    pub async fn run<H>(mut self, handler: &mut H) -> anyhow::Result<()>
    where
        H: ?Sized + Send,
        M: Dispatch<H>,
    {
        while let Some(msg) = self.rx.recv().await {
            let op = msg.op_name();
            msg.dispatch(handler)
                .await
                .map_err(|e| e.context(format!("handling {}::{op}", M::INTERFACE)))?;
        }
        Ok(())
    }

    pub fn into_inner(self) -> mpsc::UnboundedReceiver<M> {
        self.rx
    }
}

unsafe extern "C" fn dispatch_trampoline<M: Message>(
    ctx: *mut c_void,
    op: u32,
    data: *const u8,
    len: usize,
) {
    // Never unwind into C++.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ctx is the Ctx<M> registered in `create`; it outlives all callbacks.
        let ctx = unsafe { &*ctx.cast::<Ctx<M>>() };
        let bytes = if len == 0 {
            &[][..]
        } else {
            // SAFETY: the shim passes a buffer valid for `len` bytes for the duration of the call.
            unsafe { std::slice::from_raw_parts(data, len) }
        };
        let mut r = Reader::new(bytes);
        match M::decode(op, &mut r).and_then(|m| r.finish().map(|()| m)) {
            Ok(msg) => {
                trace!(interface = M::INTERFACE, op = msg.op_name(), "received");
                // Receiver gone means nobody listens anymore; dropping is correct.
                let _ = ctx.tx.send(msg);
            }
            Err(e) => {
                warn!(interface = M::INTERFACE, op, error = %e, "dropping undecodable message")
            }
        }
    }));
}

fn name_arg(name: &str) -> Result<(*const std::ffi::c_char, usize)> {
    if name.contains('\0') {
        return Err(Error::InvalidName(name.to_owned()));
    }
    Ok((name.as_ptr().cast(), name.len()))
}

impl<R: Role> Endpoint<R> {
    /// Creates the endpoint on DDS `domain` for `service_name` and returns it with its inbox.
    pub fn create(
        domain: i32,
        service_name: impl Into<String>,
    ) -> Result<(Self, Inbox<R::Incoming>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let ctx = NonNull::from(Box::leak(Box::new(Ctx { tx })));
        match Self::create_inner(domain, service_name.into(), Some(ctx)) {
            Ok(ep) => Ok((ep, Inbox { rx })),
            Err(e) => {
                // SAFETY: create failed, so the shim holds no reference to ctx.
                drop(unsafe { Box::from_raw(ctx.as_ptr()) });
                Err(e)
            }
        }
    }

    /// Creates an endpoint that only sends and never receives.
    pub fn create_write_only(domain: i32, service_name: impl Into<String>) -> Result<Self> {
        Self::create_inner(domain, service_name.into(), None)
    }

    fn create_inner(
        domain: i32,
        service_name: String,
        ctx: Option<NonNull<Ctx<R::Incoming>>>,
    ) -> Result<Self> {
        let (name, name_len) = name_arg(&service_name)?;
        let mut handle = std::ptr::null_mut();
        let (ctx_ptr, dispatch) = match ctx {
            Some(c) => (
                c.as_ptr().cast::<c_void>(),
                Some(dispatch_trampoline::<R::Incoming> as onboardapi_sys::obapi_dispatch_fn),
            ),
            None => (std::ptr::null_mut(), None),
        };
        // SAFETY: name is valid for name_len bytes; ctx stays alive until `destroy`.
        let code =
            unsafe { (R::vtable().create)(domain, name, name_len, ctx_ptr, dispatch, &mut handle) };
        check(code, || {
            format!("creating {} {service_name:?} on domain {domain}", R::NAME)
        })?;
        let handle = NonNull::new(handle).ok_or_else(|| Error::Native {
            context: format!("creating {}", R::NAME),
            code: onboardapi_sys::OBAPI_ERR_INVALID,
            message: "shim returned a null handle".into(),
        })?;
        Ok(Self {
            inner: Arc::new(Inner {
                handle,
                ctx,
                domain,
                service_name,
            }),
            _role: PhantomData,
        })
    }

    /// Sends one outgoing operation.
    pub fn send(&self, msg: &R::Outgoing) -> Result<()> {
        let mut w = Writer::default();
        msg.encode_args(&mut w);
        self.send_raw(msg.op_id(), w.as_bytes())
    }

    /// Sends pre-encoded arguments of operation `op`. Used by the generated typed senders.
    pub fn send_raw(&self, op: u32, args: &[u8]) -> Result<()> {
        // SAFETY: handle is live for &self; args is valid for its length.
        let code = unsafe {
            (R::vtable().send)(self.inner.handle.as_ptr(), op, args.as_ptr(), args.len())
        };
        check(code, || {
            format!("{} {:?}: sending op {op}", R::NAME, self.inner.service_name)
        })
    }

    /// False if the underlying DDS session was torn down.
    pub fn is_valid(&self) -> bool {
        // SAFETY: handle is live for &self.
        unsafe { (R::vtable().is_valid)(self.inner.handle.as_ptr()) }
    }

    pub fn domain(&self) -> i32 {
        self.inner.domain
    }

    pub fn service_name(&self) -> &str {
        &self.inner.service_name
    }
}

impl<R: ClientSide> Endpoint<R> {
    /// Blocks the calling thread until the matching service is discovered.
    pub fn wait_until_connected(&self) -> Result<()> {
        let f = R::vtable()
            .wait_until_connected
            .expect("client vtables provide wait_until_connected");
        // SAFETY: handle is live for &self.
        check(unsafe { f(self.inner.handle.as_ptr()) }, || {
            format!("{}: waiting for service", R::NAME)
        })
    }

    /// Async [`Self::wait_until_connected`] on tokio's blocking pool.
    ///
    /// Dropping the future does not stop the wait; wrap it in `tokio::time::timeout`
    /// only if leaving a parked blocking thread behind is acceptable.
    pub async fn connected(&self) -> Result<()> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.wait_until_connected()).await?
    }

    /// Sets the client display name shown to services and in the inspector.
    pub fn set_name(&self, name: &str) -> Result<()> {
        let f = R::vtable()
            .set_name
            .expect("client vtables provide set_name");
        let (p, n) = name_arg(name)?;
        // SAFETY: handle is live; p is valid for n bytes.
        check(unsafe { f(self.inner.handle.as_ptr(), p, n) }, || {
            format!("{}: set_name", R::NAME)
        })
    }
}
