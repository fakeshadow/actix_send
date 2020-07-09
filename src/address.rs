use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use futures_channel::oneshot::channel;
use futures_util::stream::Stream;

use crate::actor::{Actor, ActorState};
use crate::builder::{Sender, WeakSender};
use crate::context::ContextMessage;
use crate::error::ActixSendError;
use crate::object::FutureResultObjectContainer;
use crate::stream::ActorStream;
use crate::util::{future_handle::FutureHandler, runtime};

// A channel sender for communicating with actor(s).
pub struct Address<A>
where
    A: Actor + 'static,
{
    strong_count: Arc<AtomicUsize>,
    tx: Sender<A>,
    state: ActorState<A>,
    _a: PhantomData<A>,
}

impl<A> Address<A>
where
    A: Actor,
{
    pub(crate) fn new(tx: Sender<A>, state: ActorState<A>) -> Self {
        Self {
            strong_count: Arc::new(AtomicUsize::new(1)),
            tx,
            state,
            _a: PhantomData,
        }
    }

    pub fn downgrade(&self) -> WeakAddress<A> {
        WeakAddress {
            strong_count: self.strong_count.clone(),
            tx: self.tx.downgrade(),
            state: self.state.clone(),
            _a: PhantomData,
        }
    }
}

impl<A> Clone for Address<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        self.strong_count.fetch_add(1, Ordering::Release);
        Self {
            strong_count: self.strong_count.clone(),
            tx: self.tx.clone(),
            state: self.state.clone(),
            _a: PhantomData,
        }
    }
}

impl<A> Drop for Address<A>
where
    A: Actor,
{
    fn drop(&mut self) {
        if self.strong_count.fetch_sub(1, Ordering::Release) == 1 {
            self.state.shutdown();
        }
    }
}

impl<A> Address<A>
where
    A: Actor,
{
    /// Send a message to actor and await for result.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send<M>(
        &self,
        msg: M,
    ) -> Result<<M as MapResult<A::Result>>::Output, ActixSendError>
    where
        M: Into<A::Message> + MapResult<A::Result>,
    {
        let (tx, rx) = channel::<A::Result>();

        let channel_message = ContextMessage::Instant(Some(tx), msg.into());

        self.tx.send(channel_message).await?;

        let res = rx.await?;

        M::map(res)
    }

    /// Send a message to actor and ignore the result.
    pub fn do_send(&self, msg: impl Into<A::Message>) {
        let msg = ContextMessage::Instant(None, msg.into());
        let this = self.tx.clone();
        runtime::spawn(async move {
            let _ = this.send(msg).await;
        });
    }

    /// Send a message after a certain amount of delay.
    ///
    /// *. If `Address` is dropped we lose all pending messages that have not met the delay deadline.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send_later(
        &self,
        msg: impl Into<A::Message>,
        delay: Duration,
    ) -> Result<(), ActixSendError> {
        let msg = ContextMessage::Delayed(msg.into(), delay);
        self.tx.send(msg).await?;
        Ok(())
    }

    /// Send a stream to actor and return a new stream applied with `Handler::handle` method.
    ///
    /// *. Item of the stream must be actor's message type.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub fn send_stream<S, I>(&self, stream: S) -> ActorStream<A, S, I>
    where
        S: Stream<Item = I>,
        I: Into<A::Message> + MapResult<A::Result> + 'static,
    {
        ActorStream::new(stream, self.tx.clone())
    }

    /// The number of currently active actors for the given address.
    pub fn current_active(&self) -> usize {
        self.state.current_active()
    }
}

macro_rules! address_run {
    ($($send:ident)*) => {
        impl<A> Address<A>
        where
            A: Actor,
        {
            /// Run a boxed future on actor.
            ///
            /// This function use dynamic dispatches to interact with actor.
            ///
            /// It gives you flexibility in exchange of some performance
            /// (Each `Address::run` would have two more heap allocation than `Address::send`)
            #[must_use = "futures do nothing unless you `.await` or poll them"]
            pub async fn run<F, R>(&self, f: F) -> Result<R, ActixSendError>
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = R> $( + $send)* + '_>> + Send + 'static,
                R: Send + 'static,
            {
                let (tx, rx) = channel::<FutureResultObjectContainer>();

                let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();

                self.tx
                    .send(ContextMessage::InstantDynamic(Some(tx), object))
                    .await?;

                rx.await?.unpack::<R>().ok_or(ActixSendError::TypeCast)
            }

            /// Run a boxed future and ignore the result.
            pub fn do_run<F>(&self, f: F)
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = ()> $( + $send)* + '_>> + Send + 'static,
            {
                let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();
                let msg = ContextMessage::InstantDynamic(None, object);

                let this = self.tx.clone();
                runtime::spawn(async move {
                    let _ = this.send(msg).await;
                });
            }

            /// Run a boxed future after a certain amount of delay.
            ///
            /// *. If `Address` is dropped we lose all pending boxed futures that have not met the delay deadline.
            #[must_use = "futures do nothing unless you `.await` or poll them"]
            pub async fn run_later<F>(&self, delay: Duration, f: F) -> Result<(), ActixSendError>
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = ()> $( + $send)* + '_>> + Send + 'static,
            {
                let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();

                self.tx
                    .send(ContextMessage::DelayedDynamic(object, delay))
                    .await?;

                Ok(())
            }

            /// Register an interval future for actor. An actor can have multiple interval futures registered.
            ///
            /// a `FutureHandler` would return that can be used to cancel it.
            ///
            /// *. dropping the `FutureHandler` would do nothing and the interval futures will be active until the address is dropped.
            #[must_use = "futures do nothing unless you `.await` or poll them"]
            pub async fn run_interval<F>(
                &self,
                dur: Duration,
                f: F,
            ) -> Result<FutureHandler<A>, ActixSendError>
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = ()> $( + $send)* + '_>> + Send + 'static,
            {
                let (tx, rx) = channel::<FutureHandler<A>>();

                let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();

                self.tx
                    .send(ContextMessage::IntervalFutureRegister(tx, object, dur))
                    .await?;

                Ok(rx.await?)
            }
        }
    };
}

#[cfg(not(feature = "actix-runtime"))]
address_run!(Send);

#[cfg(feature = "actix-runtime")]
address_run!();

pub struct WeakAddress<A>
where
    A: Actor + 'static,
{
    strong_count: Arc<AtomicUsize>,
    tx: WeakSender<A>,
    state: ActorState<A>,
    _a: PhantomData<A>,
}

impl<A> WeakAddress<A>
where
    A: Actor,
{
    pub fn upgrade(self) -> Option<Address<A>> {
        self.tx.upgrade().map(|sender| {
            self.strong_count.fetch_add(1, Ordering::SeqCst);
            Address {
                strong_count: self.strong_count,
                tx: sender,
                state: self.state,
                _a: PhantomData,
            }
        })
    }
}

// a helper trait for map result of original messages.
// M here is auto generated ActorResult from #[actor_mod] macro.
pub trait MapResult<M>: Sized {
    type Output;
    fn map(msg: M) -> Result<Self::Output, ActixSendError>;
}
