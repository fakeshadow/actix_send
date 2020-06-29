use async_channel::SendError;
use futures::channel::oneshot::Canceled;

use crate::actors::{Actor, Message};
use crate::context::ChannelMessage;

#[derive(Debug)]
pub enum ActixSendError {
    Canceled,
    Closed,
}

impl From<Canceled> for ActixSendError {
    fn from(_err: Canceled) -> Self {
        ActixSendError::Canceled
    }
}

impl<A> From<SendError<ChannelMessage<A>>> for ActixSendError
where
    A: Actor,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    fn from(_err: SendError<ChannelMessage<A>>) -> Self {
        ActixSendError::Closed
    }
}