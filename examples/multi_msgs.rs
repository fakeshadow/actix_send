#![feature(thread_id_value)]

use crate::my_actor::*;
use actix_send::prelude::*;

// we pack all possible messages types and all handler method for one actor into a mod.
// actor_mod macro would take care for the detailed implementation.
#[actor_mod]
pub mod my_actor {
    use super::*;

    // we still need individual macro for actor/messages types and handler trait impls.

    // our actor type
    #[actor]
    pub struct MyActor {
        state1: String,
        state2: String,
    }

    // we have multiple type of messages

    #[message(result = "u8")]
    pub struct Message1 {
        pub from: String,
    }

    #[message(result = "u16")]
    pub struct Message2(pub u32);

    // we impl handler trait for all message types
    // The compiler would complain if there are message types don't have an according Handler trait impl.

    #[handler]
    impl Handler for MyActor {
        // The msg and handle's return type must match former message macro's result type.
        async fn handle(&mut self, msg: Message1) -> u8 {
            println!("Actor State1 : {}", self.state1);
            println!("We got an Message1.\r\n, from: {}", msg.from);
            8
        }
    }

    #[handler]
    impl Handler for MyActor {
        async fn handle(&mut self, msg: Message2) -> u16 {
            println!("Actor State2 : {}", self.state2);
            println!("We got an Message2.\r\n, size: {}", msg.0);
            16
        }
    }

    /*
       One of the biggest limitation of this crate is when using multiple messages for an actor.

       The result type of every message MUST be different from one another.

       If you have different messages return the same type of data the macro would simply fail to generate correct code.

       You can use a wrapper type to contain the same result type. As long as the outer type is different then the macro will work.
    */

    // Since we can't return u16 as result again we wrap it in another type.
    #[message(result = "WrapperU16")]
    pub struct Message3;

    pub struct WrapperU16(pub u16);

    #[handler]
    impl Handler for MyActor {
        async fn handle(&mut self, msg: Message3) -> WrapperU16 {
            println!("We got an Message3.\r\n");
            WrapperU16(1616)
        }
    }
}

#[tokio::main]
async fn main() {
    let state1 = String::from("running");
    let state2 = String::from("running");

    // create an actor instance. The args passed to create function are in the same order and type of your Actor's struct fields.
    let actor = MyActor::create(state1, state2);

    // build and start the actor(s).
    let address = actor.build().num(1).start();

    // construct new messages.
    let msg = Message1 {
        from: "a simple test".to_string(),
    };
    let msg2 = Message2(22);
    let msg3 = Message3;

    // use address to send messages to actor and await on result.
    // We need infer our type here. and the type should be the message's result type in #[message] macro

    let res: Result<u8, ActixSendError> = address.send(msg).await;
    let res = res.unwrap();

    let res2: u16 = address.send(msg2).await.unwrap();

    let res3: WrapperU16 = address.send(msg3).await.unwrap();

    println!("We got result for Message1\r\nResult is: {}", res);
    println!("We got result for Message2\r\nResult is: {}", res2);
    println!("We got result for Message3\r\nResult is: {}", res3.0);
}
