use std::thread;

use calloop::{
    channel::{self, Sender},
    EventLoop,
};
use system_tray::{client::Client, event::Event};

fn tokio_run(s: Sender<Event>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.handle().spawn(async move {
            let client = Client::new(s).await.unwrap();
        });

        rt.block_on(std::future::pending::<()>());
    });
}

fn main() {
    let mut eventloop = EventLoop::try_new().unwrap();
    let (s, r) = channel::channel();
    eventloop
        .handle()
        .insert_source(r, |e: calloop::channel::Event<Event>, _, _| {
            println!("{e:?}\n");
        });

    tokio_run(s);

    loop {
        eventloop.dispatch(None, &mut ()).unwrap()
    }
}
