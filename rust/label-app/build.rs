// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

fn main() {
    // Embed the app icon into the Windows exe (the committed artifact
    // at windows/AppIcon.ico); other platforms need nothing here.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../../windows/AppIcon.ico")
        .compile()
        .expect("embed windows icon");
}
