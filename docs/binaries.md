# Executables and libraries

`scat FILE` on an executable, library, object file, static archive or WebAssembly module shows a summary table, then a short [hex dump](rendering.md#hex-dump) of the first bytes (256 by default).

```
/usr/bin/ls  PIE executable, x86-64
  format           ELF 64-bit LSB
  type             PIE executable
  architecture     x86-64
  entry point      0x5620
  interpreter      /lib64/ld-linux-x86-64.so.2
  linking          dynamic
  needed           libcap.so.2, libc.so.6
  stripped         yes
  dynamic symbols  15 exported, 116 imported
  sections         29
  largest          .text 103.8 KiB, .rodata 20.8 KiB, .eh_frame 11.9 KiB, ...
  security         NX yes, PIE yes, RELRO full, canary yes, FORTIFY yes
  build id         d6281af586ce362d9c1029002636397df0cf21f1
  compiler         GCC: (GNU) 16.2.1 20260810

00000000  7f 45 4c 46 02 01 01 00  00 00 00 00 00 00 00 00  │·ELF············│
…
```

## Formats

| Format | What the table shows |
|---|---|
| ELF | class and byte order, architecture, type (executable, PIE executable, shared library, relocatable object, core dump), entry point, interpreter, static or dynamic, `NEEDED` libraries, `SONAME`, `RPATH`/`RUNPATH`, stripped or not (is there a symbol table), exported and imported dynamic symbols, section count and the five largest sections, security flags, GNU build id, Go marker, `.comment` compiler strings |
| PE (`.exe`, `.dll`, `.sys`, `.efi`) | PE32 or PE32+, EXE or DLL, machine, subsystem (GUI, console, EFI, ...), entry point, .NET (CLR) marker, build timestamp, imported DLLs, export count, section count |
| Mach-O | 32 or 64 bit, file type, CPU, entry point, install name, linked dylibs, rpaths, segment count. Universal (fat) binaries list every slice with its CPU and size, then the details of the first slice |
| WebAssembly | module or component, version, function, import and export counts with the first five names, memory limits, `start` function, custom section names and the `producers` section |
| `ar` archive (`.a`) | member count, architecture of the first object file, member names. A Debian package (`.deb`, also an `ar` file) is not summarized |

The ELF security row reads the program headers and the imported symbols:

- **NX**: the `PT_GNU_STACK` segment is not executable;
- **PIE**: the file is a position independent executable (`n/a` for libraries and objects);
- **RELRO**: `full` with a `PT_GNU_RELRO` segment and immediate binding, `partial` with only the segment;
- **canary**: the file imports `__stack_chk_fail`;
- **FORTIFY**: the file imports a `__*_chk` function.

Static executables are judged by their symbol table, so a stripped static binary can show `canary no` although it has one.

## Limits and fallbacks

- Lists are cut to 20 entries with `… N more`.
- Names from the file (libraries, symbols, sections) are untrusted. They pass through the same sanitizer as every other table, so escape sequences show as `^[`.
- The whole file is read to parse it, so files over 256 MiB get the plain hex dump.
- A file that does not parse (truncated header, damaged sections) is shown as a plain hex dump, without an error. The fallback uses `[hexdump] max_bytes` (512 by default).
- Piped output is still the file's bytes, like `cat`.

## Choosing the view

`renderer = "binary"` in a [rule](configuration.md) uses this view, `renderer = "hex"` shows the hex dump only. `scat --raw FILE` prints the bytes.

```toml
[binary]
hex_bytes = 128   # hex dump length under the summary
```

Parsing is done by [goblin](https://crates.io/crates/goblin) (ELF, PE, Mach-O, `ar`) and [wasmparser](https://crates.io/crates/wasmparser) (WebAssembly), both pure Rust.
