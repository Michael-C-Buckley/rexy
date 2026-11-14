# Third-party notices

Rex Lite is an unofficial client for Rex. It is not affiliated with Superlogical.

The Rex terminal metadata parser and parts of color/cell/cursor conversion in `src/terminal.rs` are adapted from [rex-rune](https://github.com/shuv1337/rex-rune), snapshot `e914b1987681d941ddde537b37dd07acbbb146f2`. The transport and layout implementation also used that project's interoperability behavior and [rexrs](https://github.com/a10y/rexrs) as protocol references. No Rune UI code is included.

## Adapted rex-rune code

MIT License

Copyright (c) 2026 shuv

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Libraries

libghostty is used through the `ghostty-vt` Rust bindings. egui/eframe supplies the desktop interface. These and the remaining Cargo dependencies retain their respective upstream licenses; exact versions and sources are recorded in `Cargo.lock`.
