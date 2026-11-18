use std::time::Duration;

use bevy::{
    prelude::*,
    time::{Real, TimeSystems, Virtual, time_system, update_virtual_time},
};

pub struct SourceWorldTimePlugin {
    pub minimum: Duration,
    pub maximum: Duration,
}

#[derive(Resource)]
struct WorldTimeBounds {
    minimum: Duration,
    maximum: Duration,
}

#[derive(Resource, Default)]
struct PreviousVirtualTime(Option<Time<Virtual>>);

impl Plugin for SourceWorldTimePlugin {
    fn build(&self, app: &mut App) {
        assert!(
            !self.minimum.is_zero() && self.minimum <= self.maximum,
            "world time bounds are invalid"
        );
        app.world_mut()
            .remove_resource::<bevy::time::TimeReceiver>();
        app.init_resource::<Time<Virtual>>()
            .insert_resource(WorldTimeBounds {
                minimum: self.minimum,
                maximum: self.maximum,
            })
            .init_resource::<PreviousVirtualTime>();
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(self.maximum);
        app.add_systems(First, capture_virtual_time.before(TimeSystems))
            .add_systems(
                First,
                apply_world_time_bounds
                    .in_set(TimeSystems)
                    .after(time_system),
            );
    }
}

fn capture_virtual_time(clock: Res<Time<Virtual>>, mut previous: ResMut<PreviousVirtualTime>) {
    previous.0 = Some((*clock).clone());
}

fn apply_world_time_bounds(
    bounds: Res<WorldTimeBounds>,
    real: Res<Time<Real>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut current: ResMut<Time>,
    mut previous: ResMut<PreviousVirtualTime>,
) {
    let mut clock = previous
        .0
        .take()
        .expect("virtual time must be captured before the time system");
    let speed = virtual_time.relative_speed_f64();
    clock.set_relative_speed_f64(speed);
    if virtual_time.is_paused() {
        clock.pause();
    } else {
        clock.unpause();
    }
    clock.set_max_delta(Duration::MAX);
    let mut bounded_real = (*real).clone();
    bounded_real.advance_by(real.delta().clamp(bounds.minimum, bounds.maximum));
    update_virtual_time(&mut current, &mut clock, &bounded_real);
    let maximum = bounds.maximum.mul_f64(speed);
    clock.set_max_delta(if maximum.is_zero() {
        bounds.maximum
    } else {
        maximum
    });
    *virtual_time = clock;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn automatic_game_time_uses_the_game_thread_clock() {
        let (render_sender, game_receiver) = bevy::time::create_time_channels();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(game_receiver)
            .add_plugins(SourceWorldTimePlugin {
                minimum: Duration::from_micros(500),
                maximum: Duration::from_millis(400),
            });
        assert!(matches!(
            render_sender
                .0
                .try_send(bevy::platform::time::Instant::now()),
            Err(bevy::time::TrySendError::Disconnected(_))
        ));
    }

    #[test]
    fn world_bounds_clamp_raw_time_before_dilation_and_preserve_pause() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(SourceWorldTimePlugin {
                minimum: Duration::from_micros(500),
                maximum: Duration::from_millis(400),
            });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        app.update();
        for (raw, speed, expected) in [
            (Duration::from_micros(100), 1.0, Duration::from_micros(500)),
            (Duration::from_millis(200), 1.0, Duration::from_millis(200)),
            (Duration::from_millis(300), 1.0, Duration::from_millis(300)),
            (Duration::from_millis(500), 1.0, Duration::from_millis(400)),
            (Duration::from_millis(500), 0.25, Duration::from_millis(100)),
            (Duration::from_millis(300), 2.0, Duration::from_millis(600)),
        ] {
            app.insert_resource(TimeUpdateStrategy::ManualDuration(raw));
            app.world_mut()
                .resource_mut::<Time<Virtual>>()
                .set_relative_speed(speed);
            let before = app.world().resource::<Time<Virtual>>().elapsed();
            app.update();
            let virtual_time = app.world().resource::<Time<Virtual>>();
            assert_eq!(app.world().resource::<Time<Real>>().delta(), raw);
            assert_eq!(virtual_time.delta(), expected, "raw={raw:?} speed={speed}");
            assert_eq!(virtual_time.elapsed(), before + expected);
            assert_eq!(virtual_time.effective_speed(), speed);
            assert_eq!(
                virtual_time.max_delta(),
                Duration::from_millis(400).mul_f32(speed)
            );
            assert_eq!(app.world().resource::<Time>().delta(), expected);
        }
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        let before = app.world().resource::<Time<Virtual>>().elapsed();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::ZERO
        );
        assert_eq!(app.world().resource::<Time<Virtual>>().elapsed(), before);
        assert!(app.world().resource::<Time<Virtual>>().was_paused());
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(600)
        );
    }
}
