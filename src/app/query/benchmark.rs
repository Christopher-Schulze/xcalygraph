//! Opt-in query-compute measurement over an existing real index.

use std::error::Error;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

use super::{Filter, find, names, open, show};
use crate::cli::Options;
use crate::infra::store::Store;

#[test]
#[ignore = "requires an isolated prebuilt index and question set"]
fn measure_query_compute() -> Result<(), Box<dyn Error>> {
    let store = PathBuf::from(std::env::var_os("XCALYGRAPH_BENCH_STORE").ok_or("missing store")?);
    let questions = std::env::var_os("XCALYGRAPH_BENCH_QUESTIONS").ok_or("missing questions")?;
    let samples: usize = std::env::var("XCALYGRAPH_BENCH_SAMPLES")?.parse()?;
    if !(1..=1000).contains(&samples) {
        return Err("samples must be between 1 and 1000".into());
    }
    let index = open(&Store::new(store))?;
    let filter = Filter::new(&Options::default())?;
    let show_options = show::ShowOptions {
        depth: 1,
        all: false,
        json: false,
        budget: 1200,
    };
    let data = std::fs::read_to_string(questions)?;
    println!(
        "question\tfind_median_ms\tfind_p95_ms\tfind_max_ms\tshow_median_ms\tshow_p95_ms\tshow_max_ms"
    );
    let mut count = 0;
    for line in data.lines().filter(|line| !line.is_empty()) {
        let (question, name) = line.split_once('\t').ok_or("invalid question line")?;
        if question == "question" {
            continue;
        }
        let mut find_times = Vec::with_capacity(samples);
        let mut show_times = Vec::with_capacity(samples);
        #[cfg(target_os = "macos")]
        let (mut find_cpu_times, mut show_cpu_times) =
            (Vec::with_capacity(samples), Vec::with_capacity(samples));
        for iteration in 0..samples + 2 {
            let started = Instant::now();
            #[cfg(target_os = "macos")]
            let cpu_started = thread_cpu_time()?;
            let hits = find::search(&index, &filter, black_box(question));
            if hits.is_empty() {
                return Err(format!("no find results for {question}").into());
            }
            let answer = find::render(&index, &hits, false, 600);
            drop(black_box(answer));
            drop(black_box(hits));
            let find_elapsed = started.elapsed().as_nanos();
            #[cfg(target_os = "macos")]
            let find_cpu = thread_cpu_time()?
                .checked_sub(cpu_started)
                .ok_or("CPU clock went backwards")?;

            let started = Instant::now();
            #[cfg(target_os = "macos")]
            let cpu_started = thread_cpu_time()?;
            let node = names::resolve_one(
                &index,
                &filter,
                black_box(name),
                false,
                names::AMBIGUITY_BUDGET,
            )?;
            let answer = show::render(&index, &filter, node, &show_options);
            drop(black_box(answer));
            let show_elapsed = started.elapsed().as_nanos();
            #[cfg(target_os = "macos")]
            let show_cpu = thread_cpu_time()?
                .checked_sub(cpu_started)
                .ok_or("CPU clock went backwards")?;
            if iteration >= 2 {
                find_times.push(find_elapsed);
                show_times.push(show_elapsed);
                #[cfg(target_os = "macos")]
                {
                    find_cpu_times.push(u128::from(find_cpu));
                    show_cpu_times.push(u128::from(show_cpu));
                }
            }
        }
        let (find_median, find_p95, find_max) = summary(&mut find_times);
        let (show_median, show_p95, show_max) = summary(&mut show_times);
        println!(
            "{question}\t{find_median:.3}\t{find_p95:.3}\t{find_max:.3}\t{show_median:.3}\t{show_p95:.3}\t{show_max:.3}"
        );
        #[cfg(target_os = "macos")]
        {
            let (find_median, find_p95, find_max) = summary(&mut find_cpu_times);
            let (show_median, show_p95, show_max) = summary(&mut show_cpu_times);
            println!(
                "cpu\t{question}\t{find_median:.3}\t{find_p95:.3}\t{find_max:.3}\t{show_median:.3}\t{show_p95:.3}\t{show_max:.3}"
            );
        }
        count += 1;
    }
    if count == 0 {
        return Err("no questions".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn thread_cpu_time() -> std::io::Result<u64> {
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock_id: std::ffi::c_int) -> u64;
    }
    const CLOCK_THREAD_CPUTIME_ID: std::ffi::c_int = 16;
    // SAFETY: this documented clock ID is valid and the function takes no pointers.
    let nanoseconds = unsafe { clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID) };
    if nanoseconds == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(nanoseconds)
}

#[cfg(target_os = "macos")]
#[test]
fn thread_cpu_clock_advances_for_work_but_not_sleep() -> Result<(), Box<dyn Error>> {
    let started = thread_cpu_time()?;
    std::thread::sleep(std::time::Duration::from_millis(30));
    let after_sleep = thread_cpu_time()?;
    assert!(after_sleep - started < 10_000_000);
    for value in 0..100_000 {
        black_box(value);
    }
    assert!(thread_cpu_time()? > after_sleep);
    Ok(())
}

fn summary(samples: &mut [u128]) -> (f64, f64, f64) {
    samples.sort_unstable();
    let milliseconds = |nanoseconds: u128| nanoseconds as f64 / 1_000_000.0;
    (
        milliseconds(samples[samples.len() / 2]),
        milliseconds(samples[(samples.len() * 95).div_ceil(100) - 1]),
        milliseconds(samples[samples.len() - 1]),
    )
}
