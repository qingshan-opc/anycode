// CEF requires NSApplication to implement CefAppProtocol (isHandlingSendEvent /
// setHandlingSendEvent). Tauri uses a plain NSApplication, so we add the methods
// via a category and swizzle -sendEvent: (same approach as JCEF).

#import <AppKit/AppKit.h>
#import <objc/runtime.h>

static BOOL g_anycode_handling_send_event = NO;

@interface NSApplication (AnycodeCefAppProtocol)
- (BOOL)isHandlingSendEvent;
- (void)setHandlingSendEvent:(BOOL)handlingSendEvent;
- (void)anycode_cef_swizzled_sendEvent:(NSEvent *)event;
@end

@implementation NSApplication (AnycodeCefAppProtocol)

+ (void)load {
  Method original = class_getInstanceMethod(self, @selector(sendEvent:));
  Method swizzled =
      class_getInstanceMethod(self, @selector(anycode_cef_swizzled_sendEvent:));
  if (original && swizzled) {
    method_exchangeImplementations(original, swizzled);
  }
}

- (BOOL)isHandlingSendEvent {
  return g_anycode_handling_send_event;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  g_anycode_handling_send_event = handlingSendEvent;
}

- (void)anycode_cef_swizzled_sendEvent:(NSEvent *)event {
  BOOL previous = g_anycode_handling_send_event;
  g_anycode_handling_send_event = YES;
  // After exchange, this invokes the original -sendEvent:.
  [self anycode_cef_swizzled_sendEvent:event];
  g_anycode_handling_send_event = previous;
}

@end

void anycode_cef_force_link_app_protocol(void) {
  // Ensure this object file is not dead-stripped from the static archive.
  (void)[NSApplication class];
}
