//! Handles the user input which is dispatched by the main event loop.

use std::time::Duration;

use cgmath::Vector2;
use maplibre::context::MapContext;
use winit::event::{DeviceEvent, KeyEvent, TouchPhase, WindowEvent};

use crate::input::{
    camera_handler::CameraHandler, debug_handler::DebugHandler, pan_handler::PanHandler,
    pinch_handler::PinchHandler, query_handler::QueryHandler, shift_handler::ShiftHandler,
    zoom_handler::ZoomHandler,
};

mod camera_handler;
mod debug_handler;
mod pan_handler;
mod pinch_handler;
mod query_handler;
mod shift_handler;
mod zoom_handler;

pub struct InputController {
    /// Active touches by id, in logical window coordinates.
    touches: Vec<(u64, Vector2<f64>)>,
    pinch_handler: PinchHandler,
    pan_handler: PanHandler,
    zoom_handler: ZoomHandler,
    camera_handler: CameraHandler,
    shift_handler: ShiftHandler,
    query_handler: QueryHandler,
    debug_handler: DebugHandler,
}

impl InputController {
    /// Creates a new input controller.
    ///
    /// # Arguments
    ///
    /// * `speed`: How fast animation should go. Default is 1.0. 2.0 is a speedup of 2.
    /// * `sensitivity`: How much impact an action has. Default is 10px for pressing the forward
    /// key for example.
    ///
    /// returns: InputController
    ///
    pub fn new(speed: f64, sensitivity: f64, zoom_sensitivity: f64) -> Self {
        Self {
            touches: Vec::new(),
            pinch_handler: PinchHandler::default(),
            pan_handler: PanHandler::default(),
            zoom_handler: ZoomHandler::new(zoom_sensitivity),
            camera_handler: CameraHandler::new(sensitivity),
            shift_handler: ShiftHandler::new(speed, sensitivity),
            query_handler: QueryHandler::new(),
            debug_handler: DebugHandler::default(),
        }
    }

    pub fn device_input(&mut self, _event: &DeviceEvent) -> bool {
        false
    }

    /// Process the given winit `[winit::event::WindowEvent]`.
    /// Returns true if the event has been processed and false otherwise.
    pub fn window_input(&mut self, event: &WindowEvent, scale_factor: f64) -> bool {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let position: (f64, f64) = position.to_owned().into();
                let position = Vector2::from(position) / scale_factor;
                self.pan_handler.process_window_position(&position, false);
                self.query_handler.process_window_position(&position, false);
                self.zoom_handler.process_window_position(&position, false);
                self.camera_handler
                    .process_window_position(&position, false);
                true
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state, logical_key, ..
                },
                ..
            } => {
                self.shift_handler.process_key_press(logical_key, *state)
                    || self.debug_handler.process_key_press(logical_key, *state)
                    || self.zoom_handler.process_key_press(logical_key, *state)
            }
            WindowEvent::Touch(touch) => {
                let position: (f64, f64) = touch.location.to_owned().into();
                let position = Vector2::from(position) / scale_factor;
                match touch.phase {
                    TouchPhase::Started => {
                        self.touches.push((touch.id, position));
                        self.query_handler.process_touch_start();
                    }
                    TouchPhase::Moved => {
                        if let Some(entry) = self.touches.iter_mut().find(|(id, _)| *id == touch.id)
                        {
                            entry.1 = position;
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.touches.retain(|(id, _)| *id != touch.id);
                        self.query_handler.process_touch_end();
                    }
                }
                self.dispatch_touches(touch.phase == TouchPhase::Moved);
                true
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.shift_handler.process_scroll(delta);
                self.zoom_handler.process_scroll(delta);
                true
            }
            WindowEvent::MouseInput { button, state, .. } => {
                self.pan_handler.process_mouse_key_press(button, state);
                self.query_handler.process_mouse_key_press(button, state);
                self.camera_handler.process_mouse_key_press(button, state);
                true
            }
            _ => false,
        }
    }
}

impl InputController {
    /// One finger pans, two fingers pinch. `moved` is false when the set of fingers changed, in
    /// which case the remaining gesture restarts from the current positions to avoid jumps.
    fn dispatch_touches(&mut self, moved: bool) {
        match self.touches.as_slice() {
            [] => {
                self.pan_handler.process_touch_end();
                self.pinch_handler.process_touch_end();
            }
            [(_, position)] => {
                let position = *position;
                self.pinch_handler.process_touch_end();
                if !moved {
                    self.pan_handler.process_touch_end();
                    self.pan_handler.process_touch_start(&position);
                }
                self.pan_handler.process_window_position(&position, true);
                self.query_handler.process_window_position(&position, true);
                self.zoom_handler.process_window_position(&position, true);
                self.camera_handler.process_window_position(&position, true);
            }
            [(_, a), (_, b), ..] => {
                let (a, b) = (*a, *b);
                self.pan_handler.process_touch_end();
                if !moved {
                    self.pinch_handler.process_touch_end();
                }
                self.pinch_handler.process_touches(a, b);
            }
        }
    }
}

pub trait UpdateState {
    fn update_state(&mut self, state: &mut MapContext, dt: Duration);
}

impl UpdateState for InputController {
    fn update_state(&mut self, map_context: &mut MapContext, dt: Duration) {
        self.pan_handler.update_state(map_context, dt);
        self.pinch_handler.update_state(map_context, dt);
        self.zoom_handler.update_state(map_context, dt);
        self.camera_handler.update_state(map_context, dt);
        self.shift_handler.update_state(map_context, dt);
        self.query_handler.update_state(map_context, dt);
        self.debug_handler.update_state(map_context, dt);
    }
}
