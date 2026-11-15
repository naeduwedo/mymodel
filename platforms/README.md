# platforms/

Native UI shells (Task 3). Each directory holds one platform implementation;
none is started yet, and no shell is listed as available in the product until
it can actually connect to the core.

| Directory | Stack | Notes |
| --- | --- | --- |
| `windows/` | WinUI 3, Windows App SDK, C#/XAML | Requires the Windows App SDK workload and .NET; not buildable from the Rust workspace alone. |
| `linux/` | GTK4 + Libadwaita, gtk4-rs | Requires development headers for GTK 4 and libadwaita; integrates via a Cargo workspace member when it lands. |
| `macos/` | SwiftUI + AppKit | Requires Xcode; separate package, not part of `cargo build`. |

UI rules that apply to all three: no business logic, no direct file or GPU
access, and every capability gate comes from the core's feature list.
