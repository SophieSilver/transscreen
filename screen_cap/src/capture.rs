use scrap::{Capturer, Display};
use std::{io, ops::Deref};
use utils::{
    multibuffer::{MultiBuffer, MultiBufferView},
    threading::{ThreadLoop, ThreadWork},
};

use crate::frame::{FrameError, FrameGuard};

// capturer that will be working in the ThreadLoop
struct CaptureWorker {
    capturer: Capturer,
    frame_buf: MultiBuffer<Vec<u8>>,
}

impl CaptureWorker {
    fn new(display: Display, frame_buf: MultiBuffer<Vec<u8>>) -> io::Result<Self> {
        Ok(Self {
            capturer: Capturer::new(display)?,
            frame_buf,
        })
    }

    fn update(&mut self) -> Result<(), FrameError> {
        let frame = match self.capturer.frame() {
            Ok(f) => f,
            Err(e) => return Err(e.into()),
        };

        self.frame_buf.back_mut().clear();
        self.frame_buf.back_mut().extend_from_slice(&frame);
        self.frame_buf.swap();

        Ok(())
    }
}

impl ThreadWork for CaptureWorker {
    type WorkResult = Result<(), FrameError>;

    #[inline]
    fn work(&mut self) -> Self::WorkResult {
        self.update()
    }
}

pub struct ThreadedCapturer {
    thread_loop: ThreadLoop<CaptureWorker>,
    frame_buf: MultiBufferView<Vec<u8>>,
}

impl ThreadedCapturer {
    pub fn new<F>(mut display_factory: F, target_rate: f64) -> Self
    where
        F: FnMut() -> Display + Send + 'static,
    {
        let display = display_factory();
        let width = display.width();
        let height = display.height();

        let frame_buf = vec![0_u8; width * height * 4];
        let frame_buf = MultiBuffer::new(frame_buf);
        let frame_buf_reader = frame_buf.view();

        let worker_factory = move || {
            // no way to propagate that error for now
            // so we just halt and catch fire
            CaptureWorker::new(display_factory(), frame_buf).unwrap()
        };

        let thread_loop = ThreadLoop::new(worker_factory, target_rate);

        Self {
            thread_loop,
            frame_buf: frame_buf_reader,
        }
    }

    pub fn frame(&mut self) -> Result<impl Deref<Target = [u8]> + '_, FrameError> {
        // waits for the frame and bubbles up the error if there is one
        self.thread_loop.work_recv().unwrap()?;

        // lock the frame buf
        let frame_guard = FrameGuard::new(self.frame_buf.front());

        // clear the backlog of messages and get the last error if any
        let error_iter = self.thread_loop.work_try_iter().filter_map(|message| {
            message.err().filter(|e| {
                // don't count skipped frames
                matches!(e, FrameError::Error(_))
            })
        });

        if let Some(e) = error_iter.last() {
            return Err(e);
        }

        Ok(frame_guard)
    }
}
