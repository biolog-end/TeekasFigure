use std::time::{Duration, Instant};

/// A timed notification displayed in the overlay.
/// Notifications expire after a set duration and are color-coded by severity.
pub struct Notification {
    /// The message text to display
    pub message: String,
    /// When this notification should disappear
    pub expires_at: Instant,
    /// true = red (error), false = yellow (warning)
    pub is_error: bool,
}

/// Holds all state needed to render the egui statistics overlay.
/// Updated each frame by the main loop and rendered via `render()`.
pub struct OverlayState {
    /// Current frame number (meaningful in video mode)
    pub frame_number: u32,
    /// Number of shapes successfully placed on the canvas
    pub placed_shapes: u32,
    /// Maximum shapes allowed (from settings)
    pub max_shapes: u32,
    /// Current mean squared error of the canvas vs target
    pub current_mse: f32,
    /// Frames per second (presentation rate)
    pub fps: f32,
    /// Active notifications (timed messages)
    pub notifications: Vec<Notification>,
    /// Whether we are in video mode (shows frame number)
    pub is_video: bool,
}

impl OverlayState {
    /// Create a new overlay state with default values.
    pub fn new(max_shapes: u32, is_video: bool) -> Self {
        Self {
            frame_number: 0,
            placed_shapes: 0,
            max_shapes,
            current_mse: 1.0,
            fps: 0.0,
            notifications: Vec::new(),
            is_video,
        }
    }

    /// Render the overlay panel using egui.
    /// Displays statistics in the top-left corner and notifications below.
    pub fn render(&mut self, ctx: &egui::Context) {
        self.cleanup_notifications();

        egui::Area::new(egui::Id::new("overlay_stats"))
            .fixed_pos(egui::pos2(10.0, 10.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_black_alpha(180))
                    .inner_margin(egui::Margin::same(8.0))
                    .rounding(4.0)
                    .show(ui, |ui| {
                        // Show frame number only in video mode
                        if self.is_video {
                            ui.colored_label(
                                egui::Color32::WHITE,
                                format!("Frame: {}", self.frame_number),
                            );
                        }

                        ui.colored_label(
                            egui::Color32::WHITE,
                            format!("Shapes: {} / {}", self.placed_shapes, self.max_shapes),
                        );

                        ui.colored_label(
                            egui::Color32::WHITE,
                            format!("MSE: {:.4}", self.current_mse),
                        );

                        ui.colored_label(
                            egui::Color32::WHITE,
                            format!("FPS: {:.0}", self.fps),
                        );

                        // Render notifications below stats
                        if !self.notifications.is_empty() {
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(4.0);

                            for notification in &self.notifications {
                                let color = if notification.is_error {
                                    egui::Color32::from_rgb(255, 80, 80) // Red for errors
                                } else {
                                    egui::Color32::from_rgb(255, 220, 50) // Yellow for warnings
                                };
                                ui.colored_label(color, &notification.message);
                            }
                        }
                    });
            });
    }

    /// Add a notification that will be displayed for the given duration.
    /// Maximum 5 notifications are visible at once; oldest are removed first.
    pub fn add_notification(&mut self, message: String, duration_secs: f32, is_error: bool) {
        let expires_at = Instant::now() + Duration::from_secs_f32(duration_secs);
        self.notifications.push(Notification {
            message,
            expires_at,
            is_error,
        });

        // Keep at most 5 visible notifications, removing oldest first
        while self.notifications.len() > 5 {
            self.notifications.remove(0);
        }
    }

    /// Remove expired notifications.
    pub fn cleanup_notifications(&mut self) {
        let now = Instant::now();
        self.notifications.retain(|n| n.expires_at > now);
    }
}
