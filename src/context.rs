use std::time::Duration;

use async_channel::Receiver;
use futures_channel::oneshot::Sender as OneshotSender;

use crate::actor::{Actor, ActorState, Handler};
use crate::builder::WeakSender;
use crate::object::{FutureObjectContainer, FutureResultObjectContainer};
use crate::util::future_handle::{spawn_cancelable, FutureHandler};
use crate::util::runtime;

// ActorContext would hold actor instance local state.
// State shared by actors are stored in ActorState.
pub(crate) struct ActorContext<A>
where
    A: Actor + Handler + 'static,
{
    tx: WeakSender<A>,
    rx: Receiver<ContextMessage<A>>,
    manual_shutdown: bool,
    actor: A,
    state: ActorState<A>,
}

impl<A> ActorContext<A>
where
    A: Actor + Handler,
{
    pub(crate) fn new(
        tx: WeakSender<A>,
        rx: Receiver<ContextMessage<A>>,
        actor: A,
        state: ActorState<A>,
    ) -> Self {
        Self {
            tx,
            rx,
            manual_shutdown: false,
            actor,
            state,
        }
    }

    fn delayed_msg(&self, msg: ContextMessage<A>, dur: Duration) {
        if let Some(tx) = self.tx.upgrade() {
            let handle_delay_on_shutdown = self.state.handle_delay_on_shutdown();

            let handler = spawn_cancelable(
                Box::pin(runtime::delay_for(dur)),
                move |either| async move {
                    if let futures_util::future::Either::Left(_) = either {
                        if !handle_delay_on_shutdown {
                            return;
                        }
                    }
                    let _ = tx.send(msg).await;
                },
            );

            self.state.push_handler(vec![handler]);
        }
    }

    pub(crate) fn spawn_loop(mut self) {
        runtime::spawn(async {
            self.actor.on_start();
            self.state.inc_active();

            while let Ok(msg) = self.rx.recv().await {
                match msg {
                    ContextMessage::ManualShutDown(tx) => {
                        if tx.send(()).is_ok() {
                            self.manual_shutdown = true;
                            break;
                        }
                    }
                    ContextMessage::Instant(tx, msg) => {
                        let res = self.actor.handle(msg).await;
                        if let Some(tx) = tx {
                            let _ = tx.send(res);
                        }
                    }
                    ContextMessage::InstantDynamic(tx, mut fut) => {
                        let res = fut.handle(&mut self.actor).await;
                        if let Some(tx) = tx {
                            let _ = tx.send(res);
                        }
                    }
                    ContextMessage::Delayed(msg, dur) => {
                        self.delayed_msg(ContextMessage::Instant(None, msg), dur)
                    }
                    ContextMessage::DelayedDynamic(fut, dur) => {
                        self.delayed_msg(ContextMessage::InstantDynamic(None, fut), dur)
                    }
                    ContextMessage::IntervalFutureRun(idx) => {
                        let mut guard = self.state.interval_futures.lock().await;
                        if let Some(fut) = guard.get_mut(&idx) {
                            let _ = fut.handle(&mut self.actor).await;
                        }
                    }
                    ContextMessage::IntervalFutureRemove(idx) => {
                        let _ = self.state.interval_futures.remove(idx).await;
                    }
                    ContextMessage::IntervalFutureRegister(tx, interval_future, dur) => {
                        // insert interval future to context and get it's index
                        let index = self.state.interval_futures.insert(interval_future).await;

                        // construct the interval future
                        let mut interval = runtime::interval(dur);
                        let ctx_tx = self.tx.clone();
                        let interval_loop = Box::pin(async move {
                            loop {
                                let _ = runtime::tick(&mut interval).await;
                                match ctx_tx.upgrade() {
                                    Some(tx) => {
                                        let _ =
                                            tx.send(ContextMessage::IntervalFutureRun(index)).await;
                                    }
                                    None => break,
                                }
                            }
                        });

                        // spawn a cancelable future and use the handler to execute the cancellation.
                        let mut interval_handler = spawn_cancelable(interval_loop, |_| async {});

                        // we attach the index of interval future and a tx of our channel to handler.
                        interval_handler.attach_tx(index, self.tx.clone());

                        self.state.push_handler(vec![interval_handler.clone()]);

                        let _ = tx.send(interval_handler);
                    }
                }
            }

            // dec_active will return false if the actors are already shutdown.
            if self.state.dec_active() && self.state.restart_on_err() && !self.manual_shutdown {
                return self.spawn_loop();
            };

            self.actor.on_stop();
        });
    }
}

pub(crate) enum ContextMessage<A>
where
    A: Actor,
{
    ManualShutDown(OneshotSender<()>),
    Instant(Option<OneshotSender<A::Result>>, A::Message),
    InstantDynamic(
        Option<OneshotSender<FutureResultObjectContainer>>,
        FutureObjectContainer<A>,
    ),
    Delayed(A::Message, Duration),
    DelayedDynamic(FutureObjectContainer<A>, Duration),
    IntervalFutureRegister(
        OneshotSender<FutureHandler<A>>,
        FutureObjectContainer<A>,
        Duration,
    ),
    IntervalFutureRun(usize),
    IntervalFutureRemove(usize),
}
