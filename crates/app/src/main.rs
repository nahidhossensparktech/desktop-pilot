//! Desktop Pilot binary: opens the GUI, or answers command-line flags.

mod antigravity;
mod claude_code;
mod gui;
mod mcp;
mod mcp_http;
mod platform_init;
mod training;

use std::io::Write as _;
use std::process::ExitCode;

use anyhow::Context as _;
use tracing_subscriber::EnvFilter;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Desktop Pilot - control your computer with natural language, powered by Claude.

USAGE:
    desktop-pilot [OPTIONS]

Without options, opens the Desktop Pilot window.

OPTIONS:
    -V, --version        Print version and detected platform
        --capabilities   Print which platform features are available on this machine
        --test-connection [MODEL]
                         Check the Claude credentials and send one tiny request with
                         the agent's exact settings (default model: from settings)
        --mcp            Run as an MCP server on stdin/stdout, so an MCP client such
                         as Claude Code can control this computer through Desktop
                         Pilot (no API key needed; the client provides the model)
    -h, --help           Print this help
";

fn main() -> ExitCode {
    init_logging();
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => write!(out, "{HELP}")?,
        Some("-V" | "--version") => {
            let platform = platform_init::init().context("platform detection failed")?;
            writeln!(out, "desktop-pilot {VERSION} {}", platform.info)?;
        }
        Some("--capabilities") => {
            let platform = platform_init::init().context("platform detection failed")?;
            writeln!(out, "{}", platform.info)?;
            for (name, capability) in platform.capability_report().entries {
                writeln!(out, "  {name:<24} {capability}")?;
            }
        }
        Some("--mcp-bridge") => {
            drop(out);
            mcp::run_bridge()?;
        }
        Some("--mcp") => {
            drop(out);
            let platform = platform_init::init().context("platform detection failed")?;
            tracing::info!(%platform.info, "starting MCP server on stdio");
            mcp::run(platform)?;
        }
        Some("--test-connection") => {
            drop(out);
            test_connection(args.get(1).cloned())?;
        }
        Some(other) => anyhow::bail!("unknown argument `{other}`; see --help"),
        None => {
            drop(out);
            let platform = platform_init::init().context("platform detection failed")?;
            tracing::info!(%platform.info, "starting GUI");
            gui::run(platform)?;
        }
    }
    Ok(())
}

/// `--test-connection`: credentials check plus one minimal request in the agent's
/// exact request shape.
fn test_connection(model_arg: Option<String>) -> anyhow::Result<()> {
    use std::sync::Arc;

    let mut out = std::io::stdout().lock();
    let platform = platform_init::init().context("platform detection failed")?;
    let saved_key = platform
        .secrets
        .get(dp_platform::SecretKey::AnthropicApiKey)
        .ok()
        .flatten();
    let credentials = Arc::new(dp_llm::Credentials::new(saved_key));
    match credentials.preferred_source() {
        Some(s) => writeln!(out, "credentials: {s}")?,
        None => anyhow::bail!(
            "no Claude credentials found (save a key in Settings, or run `ant auth login`)"
        ),
    }
    let client = dp_llm::ClaudeClient::new(credentials)?;
    let (settings, _) = dp_config::Settings::load();
    let model_id = model_arg.unwrap_or(settings.default_model);
    let model = dp_config::default_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .with_context(|| format!("unknown model `{model_id}`"))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    rt.block_on(async {
        let status = client.test_connection().await?;
        writeln!(out, "models endpoint: OK ({status})")?;

        let (body, betas) = dp_core::agent::probe_request(&model);
        writeln!(out, "agent-shaped request to {} (betas: {}) ...", model.id, betas.join(", "))?;
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut on_event = |_e: dp_llm::StreamEvent| {};
        let result = client.stream_message(&body, &betas, &cancel, &mut on_event).await?;
        let text: String = result
            .content
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        writeln!(
            out,
            "messages endpoint: OK, stop_reason={:?}, reply={:?}, tokens in={} out={} cache_read={} cache_write={}",
            result.stop_reason.unwrap_or_default(),
            text.trim(),
            result.usage.input_tokens,
            result.usage.output_tokens,
            result.usage.cache_read_input_tokens,
            result.usage.cache_creation_input_tokens,
        )?;
        anyhow::Ok(())
    })
}

/// Logs go to stderr for now; rolling files and secret masking arrive in M1.
/// Level from `DESKTOP_PILOT_LOG` (default `warn` for the CLI skeleton).
fn init_logging() {
    let filter =
        EnvFilter::try_from_env("DESKTOP_PILOT_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn unknown_argument_is_an_error() {
        assert!(run(vec!["--bogus".into()]).is_err());
    }

    #[test]
    fn known_arguments_succeed() {
        for arg in ["--help", "--version", "--capabilities"] {
            assert!(run(vec![arg.into()]).is_ok(), "{arg}");
        }
    }

    /// Moves the REAL mouse 200 px right and back, then restores it. Run manually with
    /// `cargo test -p desktop-pilot -- --ignored real_mouse`. Don't touch the mouse
    /// while it runs (that is detected as a takeover and fails the test).
    #[test]
    #[ignore = "moves the real mouse cursor"]
    fn real_mouse_smooth_round_trip() {
        use std::time::Duration;

        use dp_core::motion;
        use dp_platform::DesktopPoint;
        use tokio_util::sync::CancellationToken;

        let platform = crate::platform_init::init().unwrap();
        let input = platform.input.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let Some(DesktopPoint::Physical { x, y }) = input.cursor_position().await.unwrap()
            else {
                panic!("no physical cursor position on this platform");
            };
            let token = CancellationToken::new();
            // Move toward the screen interior so the target stays on a monitor.
            let monitors = platform.screen.monitors().await.unwrap();
            let m = monitors.iter().find(|m| m.is_primary).unwrap();
            let center_x = m.origin_physical.0 + i32::try_from(m.size_physical.0 / 2).unwrap();
            let dx = if x < center_x { 200 } else { -200 };
            let target = (x + dx, y);

            motion::smooth_move(&*input, target, Duration::from_millis(400), &token)
                .await
                .unwrap();
            let after = input.cursor_position().await.unwrap();
            assert_eq!(
                after,
                Some(DesktopPoint::Physical {
                    x: target.0,
                    y: target.1
                })
            );

            motion::smooth_move(&*input, (x, y), Duration::from_millis(400), &token)
                .await
                .unwrap();
            let back = input.cursor_position().await.unwrap();
            assert_eq!(back, Some(DesktopPoint::Physical { x, y }));
        });
    }
}
