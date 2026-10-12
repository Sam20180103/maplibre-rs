use std::time::Duration;

use cgmath::{InnerSpace, Vector2};
use maplibre::{context::MapContext, coords::Zoom};

use super::UpdateState;

/// Two-finger gesture: zooms by the change in finger distance around the fingers' midpoint and
/// pans with the midpoint. Positions are logical window coordinates.
#[derive(Default)]
pub struct PinchHandler {
    /// Finger positions already applied to the camera.
    applied: Option<[Vector2<f64>; 2]>,
    /// Latest finger positions.
    current: Option<[Vector2<f64>; 2]>,
}

impl UpdateState for PinchHandler {
    fn update_state(&mut self, MapContext { view_state, .. }: &mut MapContext, _dt: Duration) {
        let (Some(applied), Some(current)) = (self.applied, self.current) else {
            return;
        };
        self.applied = Some(current);

        let midpoint = |[a, b]: [Vector2<f64>; 2]| (a + b) / 2.0;
        let distance = |[a, b]: [Vector2<f64>; 2]| (a - b).magnitude();
        let (from, to) = (midpoint(applied), midpoint(current));

        // Zoom around the previous midpoint, mirroring the mouse wheel zoom.
        if distance(applied) >= 1.0 && distance(current) >= 1.0 {
            let current_zoom = view_state.zoom();
            let next_zoom =
                current_zoom + Zoom::new((distance(current) / distance(applied)).log2());
            view_state.update_zoom(next_zoom);

            let inverted_view_proj = view_state.view_projection().invert();
            if let Some(anchor) =
                view_state.window_to_world_at_ground(&from, &inverted_view_proj, false)
            {
                let scale = current_zoom.scale_delta(&next_zoom);
                view_state
                    .camera_mut()
                    .move_relative(anchor * scale - anchor);
            }
        }

        // Keep the world point under the previous midpoint under the new midpoint.
        let inverted_view_proj = view_state.view_projection().invert();
        if let (Some(from), Some(to)) = (
            view_state.window_to_world_at_ground(&from, &inverted_view_proj, false),
            view_state.window_to_world_at_ground(&to, &inverted_view_proj, false),
        ) {
            view_state.camera_mut().move_relative(from - to);
        }
    }
}

impl PinchHandler {
    pub fn process_touches(&mut self, a: Vector2<f64>, b: Vector2<f64>) {
        if self.current.is_none() {
            self.applied = Some([a, b]);
        }
        self.current = Some([a, b]);
    }

    pub fn process_touch_end(&mut self) {
        self.applied = None;
        self.current = None;
    }
}
