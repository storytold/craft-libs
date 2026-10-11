# Platform Test Inventory & Environment Matrix

This document accompanies [`platform-test-inventory.json`](platform-test-inventory.json) and defines the hardware profiles, operating system environments, and concrete behaviors validated across the Craft ecosystem.

---

## 1. Why Environment Inventories Matter

Hardware-specific defects cannot be caught by generic CI runners:
- **Issue #3**: Closing an `eframe` window crashed AppKit with `-[_NSTouchBarFinderObservation invalidate]` uncaught exceptions.
- **The Catch**: This defect **never reproduces** on desktop Macs (Mac mini, Mac Studio, Mac Pro) or non-Touch Bar laptops. Testing on a Mac mini running macOS 26 resulted in a false sense of security.
- **Resolution**: Only testing on a real physical Touch Bar MacBook Pro (`MacBookPro17,1`) running macOS 27 reproduced the crash and confirmed that `craft-quit` eliminates it.

To prevent declaring victory prematurely, shared capability authors must target the specific environment capable of exercising the invariant.

---

## 2. Environment Catalog

| Environment ID | Architecture & OS | Hardware Features | Validated Behaviors |
| :--- | :--- | :--- | :--- |
| **`macos-touchbar-m1`** | `aarch64-apple-darwin`<br>macOS 27.0.1 | MacBookPro17,1, Touch Bar, Retina | • AppKit Touch Bar KVO observation crash (`craft-quit`)<br>• Native Cmd+Q menu interception (`PR #21`)<br>• Unsaved-changes dialog veto |
| **`macos-m1pro-notouchbar`** | `aarch64-apple-darwin`<br>macOS 27.0 | MacBookPro18,1, Physical Keys | • APFS directory rename atomicity<br>• `F_FULLFSYNC` media durability<br>• `App::on_exit` state persistence |
| **`macos-macmini-m4`** | `aarch64-apple-darwin`<br>macOS 26.6.2 | Mac16,10 (Mac mini), Desktop | • Standard headless macOS compilation & clippy<br>• Clean baseline execution |
| **`linux-x86_64-ci`** | `x86_64-unknown-linux-gnu`<br>Ubuntu 24.04 LTS | Standard VM, ext4/btrfs | • POSIX rename atomicity<br>• Cross-device link errors (`EXDEV`) across `/tmp` and `/home`<br>• Parent directory fsync |
| **`windows-x86_64-ntfs`** | `x86_64-pc-windows-msvc`<br>Windows 11 23H2 | Standard VM, NTFS | • `MoveFileExW` replace semantics<br>• Mandatory file sharing violations<br>• Path case-normalization |
| **`wasm32-headless`** | `wasm32-unknown-unknown`<br>Headless | Sandboxed, No I/O | • Zero-dependency core validation<br>• Platform-agnostic trait compilation |

---

## 3. Protocol for Requesting Platform Verification

When submitting a PR that alters OS-level integration:
1. Identify whether the change affects **Filesystem Durability**, **Window Management**, or **Native Menus**.
2. Consult the inventory above to find the designated environment capable of failing under regression.
3. In the PR description, explicitly tag the required validation environment (e.g. `Requires: macos-touchbar-m1`).
4. Hardware owners record the exact model identifier, OS version, and test output in the PR review before merging.
