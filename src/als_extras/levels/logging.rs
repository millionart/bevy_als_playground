use bevy::log::{
    LogPlugin,
    tracing::{
        Event, Level, Metadata,
        field::{Field, Visit},
    },
    tracing_subscriber::{
        fmt,
        layer::{Context, Filter},
        prelude::*,
    },
};

pub(super) fn log_plugin() -> LogPlugin {
    LogPlugin {
        fmt_layer: |_| {
            Some(Box::new(
                fmt::Layer::default()
                    .with_writer(std::io::stderr)
                    .with_filter(SkipDestroyedWindowWarning),
            ))
        },
        ..Default::default()
    }
}

struct SkipDestroyedWindowWarning;

impl<S> Filter<S> for SkipDestroyedWindowWarning {
    fn enabled(&self, _: &Metadata<'_>, _: &Context<'_, S>) -> bool {
        true
    }

    fn event_enabled(&self, event: &Event<'_>, _: &Context<'_, S>) -> bool {
        let metadata = event.metadata();
        if metadata.target() != "bevy_winit::state" || *metadata.level() != Level::WARN {
            return true;
        }

        // Bevy removes the window mapping before winit delivers the last Destroyed event.
        let mut already_destroyed = DestroyedMessage(false);
        event.record(&mut already_destroyed);
        !already_destroyed.0
    }
}

struct DestroyedMessage(bool);

impl Visit for DestroyedMessage {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}")
                .starts_with("Skipped event Destroyed for unknown winit Window Id ");
        }
    }
}
