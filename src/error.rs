use async_channel::SendError;
use futures::channel::oneshot::Canceled;

use crate::actor::Actor;
use crate::context::ChannelMessage;

#[derive(Debug)]
pub enum ActixSendError {
    Canceled,
    Closed,
    Blocking,
    TypeCast,
}

impl From<Canceled> for ActixSendError {
    fn from(_err: Canceled) -> Self {
        ActixSendError::Canceled
    }
}

impl<A> From<SendError<ChannelMessage<A>>> for ActixSendError
where
    A: Actor,
{
    fn from(_err: SendError<ChannelMessage<A>>) -> Self {
        ActixSendError::Closed
    }
}
