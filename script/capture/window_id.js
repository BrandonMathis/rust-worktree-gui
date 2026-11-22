// Prints the CGWindowID of the main window owned by a process, for
// `screencapture -l`. Usage: osascript -l JavaScript window_id.js <pid>
ObjC.import('CoreGraphics');

function run(argv) {
  const pid = Number(argv[0]);
  const options = $.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements;
  const windows = ObjC.deepUnwrap(
    ObjC.castRefToObject($.CGWindowListCopyWindowInfo(options, $.kCGNullWindowID))
  );
  // Layer 0 holds normal app windows (not menus or tooltips).
  const window = windows.find((w) => w.kCGWindowOwnerPID === pid && w.kCGWindowLayer === 0);
  return window ? String(window.kCGWindowNumber) : '';
}
