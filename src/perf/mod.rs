//! Keystroke-to-pixels latency harness (NFR-11), compiled only with
//! `cargo build --features perf`.
//!
//! It drives the real app headlessly (see `harness`): a worktree is open with
//! its terminal attached, keys are pressed through iced's widget tree exactly
//! as macOS delivers them, and each one is timed until the frame's pixels
//! change. `scripts/latency.rb` runs it for two builds and compares them, and
//! `.github/workflows/latency.yml` does that on every pull request.
//!
//! ```sh
//! cargo build --release --locked --features perf
//! target/release/rust-worktree-gui --perf-latency --out latency.json
//! ```

mod echo;
mod harness;
mod scenarios;

use crate::{git, runner};
use harness::{Harness, Phases, Workspace};
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Runs the harness.
const LATENCY_FLAG: &str = "--perf-latency";
/// Runs the terminal program the harness types into (`echo`).
const ECHO_FLAG: &str = "--perf-echo";

/// The output format version; `scripts/latency.rb` checks it.
const SCHEMA: u32 = 1;

#[derive(Clone)]
pub struct Options {
    samples: usize,
    warmup: usize,
    /// The window size in logical pixels.
    window: iced::Size,
    scale: f32,
    /// `wgpu` or `tiny-skia`; `None` tries wgpu, then tiny-skia.
    renderer: Option<String>,
    only: Option<Vec<String>>,
    out: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            samples: 60,
            warmup: 10,
            // The app's default window, on a Retina display.
            window: iced::Size::new(1120.0, 720.0),
            scale: 2.0,
            renderer: None,
            only: None,
            out: None,
        }
    }
}

const USAGE: &str = "\
usage: rust-worktree-gui --perf-latency [options]
  --samples N      measured keystrokes per scenario (default 60)
  --warmup N       unmeasured keystrokes first (default 10)
  --size WxH       window size in points (default 1120x720)
  --scale F        display scale factor (default 2)
  --renderer NAME  wgpu (GPU) or tiny-skia (CPU); default: wgpu if available
  --only a,b       run only these scenarios
  --list           list the scenarios and exit
  --out PATH       write the JSON report here (default: stdout)";

/// Runs the harness or its terminal program when the command line asks for
/// one, and returns the process exit code; `None` means start the app.
pub fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next()?;
    match first.to_str()? {
        ECHO_FLAG => Some(echo::run()),
        LATENCY_FLAG => Some(
            match parse(args).and_then(|options| {
                run_as_ui_thread();
                measure(options)
            }) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("latency: {error}");
                    2
                }
            },
        ),
        _ => None,
    }
}

/// Gives this thread the scheduling class of an app's UI thread (macOS
/// "user interactive" quality of service). A command-line process gets the
/// default class, which macOS may run on the slower efficiency cores when the
/// machine is busy, making timings jump by half between runs. The app's own UI
/// thread has this class, so this is also what users experience. Threads
/// started from here (the terminal's PTY reader) inherit it.
#[cfg(target_os = "macos")]
fn run_as_ui_thread() {
    // From <sys/qos.h>.
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    // SAFETY: a libSystem function that only changes the calling thread's
    // scheduling class; both arguments are valid constants.
    let status = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
    if status != 0 {
        eprintln!(
            "latency: could not raise the thread's QoS (error {status}); timings may be noisy"
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn run_as_ui_thread() {}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.map(|arg| {
        arg.into_string()
            .map_err(|_| "arguments must be UTF-8".to_owned())
    });
    while let Some(arg) = args.next() {
        let arg = arg?;
        let mut value = || {
            args.next()
                .unwrap_or_else(|| Err(format!("{arg} needs a value\n{USAGE}")))
        };
        let number = |text: String| {
            text.parse::<usize>()
                .map_err(|_| format!("not a whole number: {text}"))
        };
        match arg.as_str() {
            "--samples" => options.samples = number(value()?)?.max(1),
            "--warmup" => options.warmup = number(value()?)?,
            "--size" => {
                let text = value()?;
                let (width, height) = text
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or_else(|| format!("--size takes WIDTHxHEIGHT, not {text}"))?;
                options.window = iced::Size::new(width, height);
            }
            "--scale" => {
                let text = value()?;
                options.scale = text
                    .parse::<f32>()
                    .ok()
                    .filter(|scale| *scale >= 1.0)
                    .ok_or_else(|| format!("--scale takes a number ≥ 1, not {text}"))?;
            }
            "--renderer" => {
                let name = value()?;
                if !["wgpu", "tiny-skia"].contains(&name.as_str()) {
                    return Err(format!("--renderer takes wgpu or tiny-skia, not {name}"));
                }
                options.renderer = Some(name);
            }
            "--only" => {
                options.only = Some(value()?.split(',').map(str::to_owned).collect());
            }
            "--out" => options.out = Some(PathBuf::from(value()?)),
            "--list" => {
                for scenario in scenarios::all() {
                    println!("{:<22} {}", scenario.name, scenario.description);
                }
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option {arg}\n{USAGE}")),
        }
    }
    Ok(options)
}

/// The JSON report: every sample, in milliseconds, for the comparison script.
#[derive(Serialize)]
struct Report {
    schema: u32,
    /// The renderer that drew the frames; only runs with the same one compare.
    renderer: String,
    window: Window,
    samples_per_scenario: usize,
    scenarios: Vec<ScenarioReport>,
}

#[derive(Serialize)]
struct Window {
    width: f32,
    height: f32,
    scale: f32,
}

#[derive(Serialize)]
struct ScenarioReport {
    name: &'static str,
    description: &'static str,
    /// Key press to changed pixels.
    total_ms: Vec<f64>,
    /// The app's own work in `total_ms`: `update_ms + view_ms + draw_ms`.
    app_ms: Vec<f64>,
    update_ms: Vec<f64>,
    view_ms: Vec<f64>,
    draw_ms: Vec<f64>,
    raster_ms: Vec<f64>,
    wait_ms: Vec<f64>,
}

impl ScenarioReport {
    fn new(scenario: &scenarios::Scenario, samples: &[Phases]) -> Self {
        let series = |phase: fn(&Phases) -> Duration| {
            samples
                .iter()
                .map(|sample| phase(sample).as_secs_f64() * 1000.0)
                .collect()
        };
        Self {
            name: scenario.name,
            description: scenario.description,
            total_ms: series(Phases::total),
            app_ms: series(Phases::app),
            update_ms: series(|p| p.update),
            view_ms: series(|p| p.view),
            draw_ms: series(|p| p.draw),
            raster_ms: series(|p| p.raster),
            wait_ms: series(|p| p.wait),
        }
    }
}

fn measure(options: Options) -> Result<(), String> {
    let all = scenarios::all();
    if let Some(only) = &options.only
        && let Some(unknown) = only
            .iter()
            .find(|name| !all.iter().any(|s| s.name == *name))
    {
        return Err(format!("unknown scenario {unknown} (see --list)"));
    }
    let fixture = tempfile::tempdir()
        .map_err(|error| format!("Could not create a temporary directory: {error}"))?;
    let workspace = workspace(fixture.path())?;
    let mut renderer = Harness::renderer_for(&options)?;
    let mut report = Report {
        schema: SCHEMA,
        renderer: iced::advanced::renderer::Headless::name(&renderer),
        window: Window {
            width: options.window.width,
            height: options.window.height,
            scale: options.scale,
        },
        samples_per_scenario: options.samples,
        scenarios: Vec::new(),
    };
    // Warm up the GPU (its clocks, shader and glyph caches) and the CPU
    // before the first measurement, with a scenario that draws a lot.
    if let Some(warmup) = all.iter().find(|s| s.name == "page_redraw") {
        let options = Options {
            samples: options.warmup * 3,
            warmup: 0,
            ..options.clone()
        };
        renderer = run(warmup, &options, &workspace, renderer)
            .map_err(|error| format!("warm-up: {error}"))?
            .0;
    }
    for scenario in all.iter().filter(|scenario| {
        options
            .only
            .as_ref()
            .is_none_or(|only| only.iter().any(|name| name == scenario.name))
    }) {
        let (returned, samples) = run(scenario, &options, &workspace, renderer)
            .map_err(|error| format!("{}: {error}", scenario.name))?;
        renderer = returned;
        let result = ScenarioReport::new(scenario, &samples);
        eprintln!(
            "{:<22} median {:>7.2} ms  p95 {:>7.2} ms  (app {:>5.2} ms)",
            scenario.name,
            percentile(&result.total_ms, 0.5),
            percentile(&result.total_ms, 0.95),
            percentile(&result.app_ms, 0.5),
        );
        report.scenarios.push(result);
    }
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("Could not encode the report: {error}"))?;
    match &options.out {
        Some(path) => std::fs::write(path, json + "\n")
            .map_err(|error| format!("Could not write {}: {error}", path.display())),
        None => {
            println!("{json}");
            Ok(())
        }
    }
}

/// Measures one scenario in a freshly started app, so one scenario's
/// terminal state never affects the next.
fn run(
    scenario: &scenarios::Scenario,
    options: &Options,
    workspace: &Workspace,
    renderer: iced::Renderer,
) -> Result<(iced::Renderer, Vec<Phases>), String> {
    let mut harness = Harness::start(options, workspace, renderer)?;
    for step in &scenario.setup {
        harness.step(*step)?;
    }
    let mut samples = Vec::with_capacity(options.samples);
    let mut warmup = options.warmup;
    for step in scenario.cycle.iter().cycle() {
        if samples.len() == options.samples {
            break;
        }
        if let Some(phases) = harness.step(*step)? {
            if warmup > 0 {
                warmup -= 1;
            } else {
                samples.push(phases);
            }
        }
    }
    Ok((harness.into_renderer(), samples))
}

/// A repository with a few linked worktrees, so the sidebar the app draws on
/// every frame looks like real use. The first worktree is opened.
fn workspace(dir: &Path) -> Result<Workspace, String> {
    let dir = dir
        .canonicalize()
        .map_err(|error| format!("Could not resolve {}: {error}", dir.display()))?;
    let root = dir.join("latency");
    std::fs::create_dir(&root)
        .map_err(|error| format!("Could not create {}: {error}", root.display()))?;
    let git = runner::find(runner::Tool::Git)?;
    let run_git = |args: &[&str]| -> Result<(), String> {
        let mut command = runner::command(&git);
        command.arg("-C").arg(&root).args([
            "-c",
            "user.name=Latency",
            "-c",
            "user.email=latency@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ]);
        let output = runner::run(command.args(args), Duration::from_secs(30))
            .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    };
    run_git(&["init", "-q", "-b", "main"])?;
    run_git(&["commit", "-qm", "initial", "--allow-empty"])?;
    for branch in ["feature-login", "fix-scrolling", "review-docs"] {
        run_git(&["worktree", "add", "-qb", branch, &format!("../{branch}")])?;
    }
    let checkout = git::discover(&root)?;
    let inventory = git::inventory(&checkout)?;
    Ok(Workspace {
        checkout,
        inventory,
        worktree: root,
    })
}

/// The `fraction` quantile of `values` (nearest rank), for the summary line.
fn percentile(values: &[f64], fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len().max(1));
    sorted.get(rank - 1).copied().unwrap_or(f64::NAN)
}
