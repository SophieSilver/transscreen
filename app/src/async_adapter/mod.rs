use std::{
    sync::{
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc,
    },
    thread,
};

use parking_lot::Mutex;
use screen_cap::record::{
    encoded_buffer::{ArcEncodedDataGuard, EncodedBufferView},
    EncodeStatus, RecordError, Recorder,
};
use tokio::sync::Notify;

type NextFlushResult = Result<(), Arc<RecordError>>;
type NextFrameResult = Result<EncodeStatus, Arc<RecordError>>;

#[derive(Debug, Clone)]
enum RecorderMessage {
    // RecordError isn't Clone, so Arc it is
    WaitForNextFlush(ReturnDestination<NextFlushResult>),
    WaitForFrame(ReturnDestination<NextFrameResult>),
}

#[derive(Debug, Default)]
struct ReturnDestination<T> {
    return_dest: Arc<Mutex<Option<T>>>,
    notify: Arc<Notify>,
}

impl<T> Clone for ReturnDestination<T> {
    fn clone(&self) -> Self {
        Self {
            return_dest: self.return_dest.clone(),
            notify: self.notify.clone(),
        }
    }
}

impl<T> ReturnDestination<T> {
    fn new() -> Self {
        Self {
            return_dest: Arc::default(),
            notify: Arc::default(),
        }
    }

    fn send_result(self, value: T) {
        *self.return_dest.lock() = Some(value);
        self.notify.notify_one();
    }

    async fn recv_result(&self) -> T {
        self.notify.notified().await;
        self.return_dest.lock().take().unwrap()
    }
}

#[derive(Debug)]
pub struct RecorderAsyncAdapter {
    data_buffer_dest: ReturnDestination<ArcEncodedDataGuard>,
    data_buffer_tx: Sender<ReturnDestination<ArcEncodedDataGuard>>,

    next_frame_dest: ReturnDestination<NextFrameResult>,
    next_flush_dest: ReturnDestination<NextFlushResult>,
    recorder_tx: Sender<RecorderMessage>,

    headers: Arc<[u8]>,
}

impl RecorderAsyncAdapter {
    pub fn new(recorder: Recorder) -> Self {
        let headers = recorder.headers().into();

        let data_buffer_dest = ReturnDestination::new();
        let next_frame_dest = ReturnDestination::new();
        let next_flush_dest = ReturnDestination::new();

        let (data_buffer_tx, data_buffer_rx) = mpsc::channel();
        let data_buffer_view = recorder.data_buffer_view();

        thread::spawn(move || data_buffer_managing_thread(data_buffer_view, data_buffer_rx));

        let (recorder_tx, recorder_rx) = mpsc::channel();
        thread::spawn(move || recorder_managing_thread(recorder, recorder_rx));

        Self {
            data_buffer_dest,
            data_buffer_tx,
            next_frame_dest,
            next_flush_dest,
            recorder_tx,
            headers,
        }
    }

    pub fn headers(&self) -> &[u8] {
        &self.headers
    }

    pub async fn data_buffer(&self) -> ArcEncodedDataGuard {
        self.data_buffer_tx
            .send(self.data_buffer_dest.clone())
            .unwrap();

        self.data_buffer_dest.recv_result().await
    }

    pub async fn wait_for_frame(&self) -> NextFrameResult {
        self.recorder_tx
            .send(RecorderMessage::WaitForFrame(self.next_frame_dest.clone()))
            .unwrap();

        self.next_frame_dest.recv_result().await
    }

    pub async fn wait_for_next_flush(&self) -> NextFlushResult {
        self.recorder_tx
            .send(RecorderMessage::WaitForNextFlush(
                self.next_flush_dest.clone(),
            ))
            .unwrap();

        self.next_flush_dest.recv_result().await
    }
}

impl Clone for RecorderAsyncAdapter {
    fn clone(&self) -> Self {
        Self {
            data_buffer_tx: self.data_buffer_tx.clone(),
            recorder_tx: self.recorder_tx.clone(),
            headers: self.headers.clone(),
            data_buffer_dest: ReturnDestination::new(),
            next_frame_dest: ReturnDestination::new(),
            next_flush_dest: ReturnDestination::new(),
        }
    }
}

// thread that blocks for the lock on the data_buffer so that your async functions don't have to
// it receives a messages with the reference to the cell where to put the acquired lock guard
// and a Notify struct that wakes up the task that sent that message
fn data_buffer_managing_thread(
    data_buffer_view: EncodedBufferView,
    rx: Receiver<ReturnDestination<ArcEncodedDataGuard>>,
) {
    // thread will terminate when the sender drops
    for dest in rx.iter() {
        let result = data_buffer_view.get_arc();

        dest.send_result(result);
    }
}

fn recorder_managing_thread(recorder: Recorder, rx: Receiver<RecorderMessage>) {
    let mut flush_waiters = Vec::new();

    loop {
        let result = recorder.wait_for_frame().map_err(Arc::new);
        // check if the channel hang up and terminate the loop if it did
        match rx.try_recv() {
            Ok(msg) => handle_recorder_message(msg, &mut flush_waiters, result.clone()),
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => (),
        }

        for msg in rx.try_iter() {
            handle_recorder_message(msg, &mut flush_waiters, result.clone());
        }

        // flush the waiters
        // needed in case we have flush but no messages currently
        // a bit of code duplication, idk what else to do
        if !result
            .as_ref()
            .is_ok_and(|&status| status != EncodeStatus::Flushed)
        {
            let mapped_result = result.map(|_| ());

            flush_waiters
                .drain(..)
                .for_each(|d: ReturnDestination<_>| d.send_result(mapped_result.clone()));
        }
    }
}

fn handle_recorder_message(
    msg: RecorderMessage,
    flush_waiters: &mut Vec<ReturnDestination<NextFlushResult>>,
    result: NextFrameResult,
) {
    match msg {
        RecorderMessage::WaitForFrame(dest) => dest.send_result(result),
        RecorderMessage::WaitForNextFlush(dest) => {
            // if it's not (Flushed or error) push it into the vec of flush waiters
            if result
                .as_ref()
                .is_ok_and(|&status| status != EncodeStatus::Flushed)
            {
                flush_waiters.push(dest);
                return;
            }

            let mapped_result = result.map(|_| ());
            dest.send_result(mapped_result.clone());

            flush_waiters
                .drain(..)
                .for_each(|d: ReturnDestination<_>| d.send_result(mapped_result.clone()));
        }
    }
}
