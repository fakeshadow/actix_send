use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread::ThreadId;

use futures_util::stream::{FuturesUnordered, StreamExt};

use actix_send::prelude::*;

use crate::non_shared_actor::NonSharedActor;
use crate::shared_actor::{Message2, SharedActor};

#[actor_mod]
pub mod shared_actor {
    use super::*;

    // Actor is shareable as long as it's state contains thread safe share able data structure.
    #[actor]
    pub struct SharedActor {
        pub state: Arc<AtomicUsize>,
        pub info: Arc<Mutex<HashMap<ThreadId, usize>>>,
    }

    #[message(result = "u8")]
    pub struct Message1;

    #[message(result = "Vec<Message2Res>")]
    pub struct Message2;

    pub struct Message2Res {
        pub thread_id: ThreadId,
        pub count: usize,
    }

    #[handler]
    impl Handler for SharedActor {
        async fn handle(&mut self, _msg: Message1) -> u8 {
            self.state.fetch_add(1, Ordering::Relaxed);
            let id = std::thread::current().id();
            let mut guard = self.info.lock().unwrap();
            let key = guard.get_mut(&id);
            match key {
                Some(key) => *key += 1,
                None => {
                    guard.insert(id, 1);
                }
            };
            1
        }
    }

    #[handler]
    impl Handler for SharedActor {
        async fn handle(&mut self, _msg: Message2) -> Vec<Message2Res> {
            println!(
                "Actor have handled a total of {} messages\r\n",
                self.state.load(Ordering::SeqCst)
            );
            self.info
                .lock()
                .unwrap()
                .iter()
                .map(|(thread_id, count)| Message2Res {
                    thread_id: *thread_id,
                    count: *count,
                })
                .collect()
        }
    }
}

#[actor_mod]
pub mod non_shared_actor {
    use super::*;

    #[actor]
    pub struct NonSharedActor {
        pub state: usize,
    }

    #[message(result = "usize")]
    pub struct Message1;

    #[handler]
    impl Handler for NonSharedActor {
        async fn handle(&mut self, _msg: Message1) -> usize {
            self.state += 1;
            self.state
        }
    }
}

#[tokio::main]
async fn main() {
    // build shared smart pointers first.
    let state = Arc::new(AtomicUsize::new(0));
    let info = Arc::new(Mutex::new(HashMap::new()));

    // move the clone of pointers to builder function.
    let builder = SharedActor::builder(move || {
        let state = state.clone();
        let info = info.clone();
        async { SharedActor { state, info } }
    });

    // set the actor instances we want with Builder::num and start them
    let address = builder.num(12).start().await;

    // send messages
    let _ = (0..1_000)
        // Both shared_actor and non_shared_actor have the same type Message1
        // and we can specific call the one we want with a type path.
        .fold(FuturesUnordered::new(), |f, _| {
            f.push(address.send(shared_actor::Message1));
            f
        })
        .collect::<Vec<_>>()
        .await;

    address
        .send(Message2)
        .await
        .unwrap()
        .into_iter()
        .for_each(|i| println!("{:?}\r\nhandled {} messages\r\n", i.thread_id, i.count));

    // build and start non share actors.
    let builder = NonSharedActor::builder(|| async { NonSharedActor { state: 0 } });

    let address2 = builder.num(12).start().await;

    // send messages
    let _ = (0..1_000)
        // Both shared_actor and non_shared_actor have the same type Message1
        // and we can specific call the one we want with a type path.
        .fold(FuturesUnordered::new(), |f, _| {
            f.push(address2.send(non_shared_actor::Message1));
            f
        })
        .collect::<Vec<_>>()
        .await;

    address2
        .send(non_shared_actor::Message1)
        .await
        .map(|state| println!("State is: {}, should be smaller than 1000", state))
        .unwrap();
}
