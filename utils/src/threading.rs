use std::{
    sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, Sender, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};

use spin_sleep::LoopHelper;

pub trait ThreadWork {
    type WorkResult: Send + 'static;

    fn work(&mut self) -> Self::WorkResult;
}

enum MessageToWorker {
    StartLoop { target_rate: f64 },
    Join,
}

struct ThreadLoopWorker<W: ThreadWork> {
    worker: W,
    tx: Sender<W::WorkResult>,
    rx: Receiver<MessageToWorker>,
}

// struct that will be running its code on another thread
impl<W: ThreadWork> ThreadLoopWorker<W> {
    fn new(worker: W, tx: Sender<W::WorkResult>, rx: Receiver<MessageToWorker>) -> Self {
        Self { worker, tx, rx }
    }

    fn run(&mut self) {
        let target_rate = match self.rx.recv().unwrap() {
            MessageToWorker::StartLoop { target_rate } => target_rate,
            MessageToWorker::Join => return,
        };

        let mut loop_helper = LoopHelper::builder()
            // .report_interval_s(1.0)      // for debugging
            .build_with_target_rate(target_rate);

        loop {
            loop_helper.loop_start();

            // handle incoming messages
            #[allow(clippy::never_loop)]
            for message in self.rx.try_iter() {
                match message {
                    // safety: start can only be called once per worker
                    // as `ThreadLoopBuilder::loop_start` takes ownership of itself
                    MessageToWorker::StartLoop { .. } => {
                        unreachable!()
                    }
                    MessageToWorker::Join => return,
                }
            }

            let result = self.worker.work();

            self.tx.send(result).unwrap();

            loop_helper.loop_sleep();
        }
    }
}

// this is here so we don't need to implement drop twice
struct ThreadLoopInner<W: ThreadWork> {
    worker_join_handle: JoinHandle<()>,
    tx: SyncSender<MessageToWorker>,
    rx: Receiver<W::WorkResult>,
}

impl<W: ThreadWork> Drop for ThreadLoopInner<W> {
    fn drop(&mut self) {
        // intentionally silencing the error if there is one
        let _ = self.tx.send(MessageToWorker::Join);
        // not joining the handle to keep the drop low cost
    }
}

pub struct ThreadLoopBuilder<W: ThreadWork> {
    inner: ThreadLoopInner<W>,
}

impl<W: ThreadWork> ThreadLoopBuilder<W> {
    pub fn new<F>(worker_factory: F) -> Self
    where
        F: FnOnce() -> W,
        F: Send + 'static,
    {
        // technically there will only ever be 2 messages sent at most
        // I'm just generous setting the value to 8
        let (tx, worker_rx) = mpsc::sync_channel::<MessageToWorker>(8);

        let (worker_tx, rx) = mpsc::channel::<W::WorkResult>();

        let worker_join_handle = thread::spawn(move || {
            let inner_worker = worker_factory();

            let mut loop_worker = ThreadLoopWorker::new(inner_worker, worker_tx, worker_rx);

            loop_worker.run();
        });

        Self {
            inner: ThreadLoopInner {
                worker_join_handle,
                tx,
                rx,
            },
        }
    }

    #[inline]
    pub fn start_loop(self, target_rate: f64) -> ThreadLoop<W> {
        self.inner
            .tx
            .send(MessageToWorker::StartLoop { target_rate })
            .unwrap();

        ThreadLoop { inner: self.inner }
    }
}

pub struct ThreadLoop<W: ThreadWork> {
    inner: ThreadLoopInner<W>,
}

impl<W: ThreadWork> ThreadLoop<W> {
    pub fn new<F>(worker_factory: F, target_rate: f64) -> Self
    where
        F: FnOnce() -> W,
        F: Send + 'static,
    {
        let builder = ThreadLoopBuilder::new(worker_factory);

        builder.start_loop(target_rate)
    }

    #[inline]
    pub fn work_try_iter(&self) -> impl Iterator<Item = W::WorkResult> + '_ {
        self.inner.rx.try_iter()
    }

    #[inline]
    pub fn work_recv(&self) -> Result<<W as ThreadWork>::WorkResult, RecvError> {
        self.inner.rx.recv()
    }

    #[inline]
    pub fn work_recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<<W as ThreadWork>::WorkResult, RecvTimeoutError> {
        self.inner.rx.recv_timeout(timeout)
    }

    #[inline]
    pub fn work_iter(&self) -> impl Iterator<Item = W::WorkResult> + '_ {
        self.inner.rx.iter()
    }

    #[inline]
    pub fn exited(&mut self) -> bool {
        self.inner.worker_join_handle.is_finished()
    }
}
