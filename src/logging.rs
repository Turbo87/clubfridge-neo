use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

const DEFAULT_TARGETS: &str = "warn,clubfridge_neo=debug";

pub fn init() -> anyhow::Result<()> {
    let targets = targets_from_env();

    let stdout_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_filter(targets.clone());

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("clubfridge-neo")
        .filename_suffix("log")
        .max_log_files(7)
        .build("logs")?;

    let logfile_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(file_appender)
        .with_filter(targets);

    Ok(tracing_subscriber::registry()
        .with(stdout_layer)
        .with(logfile_layer)
        .try_init()?)
}

fn targets_from_env() -> Targets {
    let targets = match std::env::var("RUST_LOG") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return default_targets(),
        Err(err) => {
            eprintln!("Ignoring `RUST_LOG`: {err}");
            return default_targets();
        }
    };

    targets.parse().unwrap_or_else(|err| {
        eprintln!("Ignoring `RUST_LOG={targets:?}`: {err}");
        default_targets()
    })
}

fn default_targets() -> Targets {
    DEFAULT_TARGETS.parse().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_targets_does_not_panic() {
        default_targets();
    }
}
