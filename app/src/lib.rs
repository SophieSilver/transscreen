pub mod server;
pub mod async_adapter;

use std::{
    fs::File,
    io::{BufWriter, Write},
    time::{Duration, Instant},
};

use async_adapter::RecorderAsyncAdapter;
use scrap::Display;
use screen_cap::record::{BufferingSettings, CapturerSettings, EncoderSettings, Recorder};
use spin_sleep::LoopHelper;
use tokio::{runtime::Builder, io::AsyncWriteExt};
use x264::{Colorspace, Preset, Setup, Tune};

// it seems that the real update rate is half as large
// possibly because scrap likes skipping frames
const TARGET_RATE: f64 = 120.0;
// 50 MiB
const BUFFER_CAPACITY: usize = 50 * 8 * 1024 * 1024;
const BUFFERED_FRAMES: usize = 0;
// 4 Mbits/s
const BITRATE: i32 = 4000;
const TIMEBASE: f64 = 1000.0;

const PRESET: Preset = Preset::Ultrafast;
const TUNE: Tune = Tune::Film;
const FAST_DECODE: bool = true;
const ZERO_LATENCY: bool = true;

pub fn run() {
    // record_to_file();
    let rt = Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(record_to_file_async());
}

async fn record_to_file_async() {
    let display = Display::primary().unwrap();
    let width = display.width();
    let height = display.height();

    let capturer_settings = CapturerSettings {
        display_factory: || Display::primary().unwrap(),
        target_rate: TARGET_RATE,
    };

    let buffering_settings = BufferingSettings {
        buffer_capacity: BUFFER_CAPACITY,
        buffered_frames: BUFFERED_FRAMES,
    };

    let encoder_settings = EncoderSettings {
        encoder_factory: move || {
            Setup::preset(PRESET, TUNE, FAST_DECODE, ZERO_LATENCY)
                .bitrate(BITRATE)
                .timebase(1, TIMEBASE as u32)
                .build(Colorspace::BGRA, width as _, height as _)
                .unwrap()
        },
        timebase: TIMEBASE,
    };

    let file = tokio::fs::File::create("thing.h264").await.unwrap();
    //let mut file_buf = BufWriter::with_capacity(8 * 1024 * 1024, file);
    
    let mut file_buf = tokio::io::BufWriter::with_capacity(8 * 1024 * 1024, file);
    
    let recorder = Recorder::new(capturer_settings, buffering_settings, encoder_settings);
    let recorder = RecorderAsyncAdapter::new(recorder);

    let mut last_chunk_id = 0;

    let start_time = Instant::now();

    file_buf.write_all(recorder.headers()).await.unwrap();
    
    let mut loop_helper = LoopHelper::builder().report_interval_s(1.0).build_without_target_rate();

    while start_time.elapsed() < Duration::from_secs(60) {
        loop_helper.loop_start();
        
        if let Some(fps) = loop_helper.report_rate() {
            dbg!(fps * (BUFFERED_FRAMES + 1) as f64 );
        }
        
        recorder.wait_for_next_flush().await.unwrap();
        loop_helper.loop_sleep();

        let data_buf = recorder.data_buffer().await;
        let (id_min, id_max) = data_buf.id_bounds();

        let start_id = id_min.max(last_chunk_id);

        for i in start_id..id_max {
            let frame = data_buf.get(i).unwrap();
            file_buf.write_all(frame.data()).await.unwrap();
        }

        last_chunk_id = id_max;
        
    }
    file_buf.flush().await.unwrap();
}

fn record_to_file() {
    let display = Display::primary().unwrap();
    let width = display.width();
    let height = display.height();

    let capturer_settings = CapturerSettings {
        display_factory: || Display::primary().unwrap(),
        target_rate: TARGET_RATE,
    };

    let buffering_settings = BufferingSettings {
        buffer_capacity: BUFFER_CAPACITY,
        buffered_frames: BUFFERED_FRAMES,
    };

    let encoder_settings = EncoderSettings {
        encoder_factory: move || {
            Setup::preset(PRESET, TUNE, FAST_DECODE, ZERO_LATENCY)
                .bitrate(BITRATE)
                .timebase(1, TIMEBASE as u32)
                .build(Colorspace::BGRA, width as _, height as _)
                .unwrap()
        },
        timebase: TIMEBASE,
    };

    let file = File::create("thing.h264").unwrap();
    let mut file_buf = BufWriter::with_capacity(8 * 1024 * 1024, file);

    let recorder = Recorder::new(capturer_settings, buffering_settings, encoder_settings);

    let mut last_chunk_id = 0;

    let start_time = Instant::now();

    file_buf.write_all(recorder.headers()).unwrap();
    
    let mut loop_helper = LoopHelper::builder().report_interval_s(1.0).build_without_target_rate();

    while start_time.elapsed() < Duration::from_secs(60) {
        loop_helper.loop_start();
        
        if let Some(fps) = loop_helper.report_rate() {
            dbg!(fps * (BUFFERED_FRAMES + 1) as f64 );
        }
        
        recorder.block_until_next_flush().unwrap();
        loop_helper.loop_sleep();

        let data_buf = recorder.data_buffer().unwrap();
        let (id_min, id_max) = data_buf.id_bounds();

        let start_id = id_min.max(last_chunk_id);

        for i in start_id..id_max {
            let frame = data_buf.get(i).unwrap();
            file_buf.write_all(frame.data()).unwrap();
        }

        last_chunk_id = id_max;
        
    }
    file_buf.flush().unwrap();
}
