#import <UIKit/UIKit.h>
#include <stdbool.h>
void gpui_ios_register_app(void);
void gpui_ios_run_demo(void);
void gpui_ios_set_embedded(void);
void *gpui_ios_get_window(void);
void *gpui_ios_view_controller(void *window);
void gpui_ios_layout_view(void *window);
bool gpui_ios_request_frame(void *window);
void gpui_ios_did_become_active(void *app);
void gpui_ios_will_resign_active(void *app);
// ios/src/lib.rs: one browse answer from Discovery.swift, `{"nodes":[…],"denied":false}`.
void sjel_ios_nodes(const char *json);
// ios/src/lib.rs: the text of a pairing code PairScanner.swift read, and why the camera could not
// open when it could not. Both are NUL-terminated UTF-8, valid only for the call.
void sjel_ios_scanned_pairing(const char *text);
void sjel_ios_scan_failed(const char *text);
// host/Audio.swift: the phone's output device, asked for the next block of frames. Rust renders
// exactly `frames` samples into each channel (ios/src/audio.rs).
void sjel_ios_soundscape_render(double sample_rate, float *left, float *right, unsigned int frames);
// ios/src/notify.rs: what the notification centre holds, `{"authorization":"allowed","scheduled":3}`
// after any ask; a tapped reminder's link; and the app coming back to the foreground, which is when
// the phone re-reads its replica and re-schedules. All NUL-terminated UTF-8, valid only for the call.
void sjel_ios_notifications_state(const char *json);
void sjel_ios_notification_opened(const char *json);
void sjel_ios_became_active(void);
