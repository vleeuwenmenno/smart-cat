//! Summaries of executables and libraries: ELF, PE, Mach-O, WebAssembly and `ar` archives.
//!
//! The summary is a key/value table shown above a short hex dump. Every string read from
//! the file (library names, symbols, section names) is untrusted; [`crate::kv`] sanitizes
//! values when the table is written. Lists are capped so a hostile header cannot flood the
//! terminal, and a file that does not parse gets `None` so the caller shows a plain hex dump.

use std::fmt::Write as _;
use std::io::{self, Read, Write};

use goblin::Object;
use goblin::archive::Archive;
use goblin::elf::{Elf, dynamic, header, program_header, section_header};
use goblin::mach::{Mach, MachO, cputype, header as mach_header};
use goblin::pe::PE;

use crate::hexdump;
use crate::kv;
use crate::util::human_size;

/// Hex dump length below the summary.
pub const DEFAULT_HEX_BYTES: usize = 256;
/// Files larger than this are not parsed (the whole file is read into memory).
pub const MAX_PARSE_BYTES: u64 = 256 * 1024 * 1024;
/// Entries shown from a list before "… N more".
const MAX_LIST: usize = 20;
/// Sections shown in the "largest" row.
const TOP_SECTIONS: usize = 5;
/// A fat Mach-O with more slices than this is not one (Java class files share its magic).
const MAX_FAT_SLICES: u32 = 32;
/// Symbols inspected for the security flags.
const MAX_SYMBOLS: usize = 4_000_000;

pub type Rows = Vec<(&'static str, String)>;

/// What the table shows: a heading line and its rows.
pub struct Summary {
    pub title: String,
    pub rows: Rows,
}

/// True when the first bytes look like a format this module summarizes.
pub fn wants(head: &[u8]) -> bool {
    head.starts_with(b"\x7fELF")
        || head.starts_with(b"\0asm")
        || head.starts_with(b"!<arch>\n")
        || is_macho_magic(head)
        || head.starts_with(b"MZ")
}

fn is_macho_magic(head: &[u8]) -> bool {
    let Some(magic) = head.first_chunk::<4>() else {
        return false;
    };
    matches!(
        *magic,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
    )
}

/// Summarizes `data`, or `None` when it is not a parseable binary of a known format.
pub fn summarize(data: &[u8]) -> Option<Summary> {
    if data.starts_with(b"\0asm") {
        return wasm(data);
    }
    match Object::parse(data).ok()? {
        Object::Elf(elf) => Some(elf_summary(&elf, data)),
        Object::PE(pe) => Some(pe_summary(&pe)),
        Object::Mach(Mach::Binary(macho)) => Some(macho_summary(&macho)),
        Object::Mach(Mach::Fat(multi)) => fat_summary(&multi),
        Object::Archive(archive) => archive_summary(&archive, data),
        _ => None,
    }
}

/// How the summary and the dump are drawn.
pub struct Options<'a> {
    pub name: &'a str,
    pub colors: bool,
    pub utf8: bool,
    /// Hex dump length under a summary.
    pub hex_bytes: usize,
    /// Hex dump length when the file cannot be summarized.
    pub fallback_bytes: usize,
}

/// Summary table and a short hex dump; a plain hex dump when the file does not parse
/// or is larger than [`MAX_PARSE_BYTES`].
pub fn render<R: Read, W: Write>(
    out: &mut W,
    reader: &mut R,
    total: Option<u64>,
    opts: &Options,
) -> io::Result<()> {
    let mut data = Vec::new();
    reader
        .by_ref()
        .take(MAX_PARSE_BYTES + 1)
        .read_to_end(&mut data)?;
    let too_big = data.len() as u64 > MAX_PARSE_BYTES;
    let summary = if too_big { None } else { summarize(&data) };
    let mut hex = hexdump::Options {
        colors: opts.colors,
        utf8: opts.utf8,
        max_bytes: opts.fallback_bytes,
    };
    let Some(summary) = summary else {
        let shown = data.len().min(hex.max_bytes);
        let truncated = data.len() > shown;
        return out.write_all(&hexdump::format(&data[..shown], total, truncated, &hex));
    };
    let title = format!("{}  {}", opts.name, summary.title);
    kv::write_table(out, &title, &summary.rows, opts.colors)?;
    writeln!(out)?;
    hex.max_bytes = opts.hex_bytes;
    let shown = data.len().min(hex.max_bytes);
    let truncated = data.len() > shown;
    out.write_all(&hexdump::format(&data[..shown], total, truncated, &hex))
}

// ---------------------------------------------------------------- helpers

/// "a, b, c, … 4 more": at most `limit` items.
fn list<I, S>(items: I, limit: usize) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let items: Vec<S> = items.into_iter().collect();
    let mut text = items
        .iter()
        .take(limit)
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > limit {
        let _ = write!(text, ", … {} more", items.len() - limit);
    }
    text
}

fn yes_no(flag: bool) -> String {
    if flag { "yes" } else { "no" }.to_string()
}

/// Civil date `YYYY-MM-DD HH:MM:SS UTC` for a Unix timestamp.
fn format_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

// ---------------------------------------------------------------- ELF

fn elf_machine(machine: u16) -> String {
    match machine {
        header::EM_X86_64 => "x86-64".to_string(),
        header::EM_386 => "x86 (i386)".to_string(),
        header::EM_AARCH64 => "AArch64".to_string(),
        header::EM_ARM => "ARM".to_string(),
        header::EM_RISCV => "RISC-V".to_string(),
        header::EM_PPC64 => "PowerPC 64".to_string(),
        header::EM_PPC => "PowerPC".to_string(),
        header::EM_S390 => "s390".to_string(),
        header::EM_MIPS => "MIPS".to_string(),
        other => format!("machine {other}"),
    }
}

fn elf_os_abi(abi: u8) -> Option<&'static str> {
    match abi {
        0 => None,
        3 => Some("Linux"),
        6 => Some("Solaris"),
        9 => Some("FreeBSD"),
        12 => Some("OpenBSD"),
        _ => Some("other"),
    }
}

/// Kind of ELF file, as people name it.
fn elf_kind(elf: &Elf) -> &'static str {
    match elf.header.e_type {
        header::ET_REL => "relocatable object",
        header::ET_EXEC => "executable",
        header::ET_CORE => "core dump",
        header::ET_DYN => {
            let pie_flag = elf
                .dynamic
                .as_ref()
                .is_some_and(|d| d.info.flags_1 & dynamic::DF_1_PIE != 0);
            // libc.so.6 names an interpreter too, but has a SONAME and no PIE flag.
            if pie_flag || (elf.interpreter.is_some() && elf.soname.is_none()) {
                "PIE executable"
            } else {
                "shared library"
            }
        }
        _ => "unknown type",
    }
}

fn section_name<'a>(elf: &'a Elf, section: &section_header::SectionHeader) -> &'a str {
    elf.shdr_strtab.get_at(section.sh_name).unwrap_or("?")
}

/// The file bytes of a section, capped at `max` bytes.
fn section_bytes<'a>(
    data: &'a [u8],
    section: &section_header::SectionHeader,
    max: usize,
) -> Option<&'a [u8]> {
    if section.sh_type == section_header::SHT_NOBITS {
        return None;
    }
    let start = usize::try_from(section.sh_offset).ok()?;
    let size = usize::try_from(section.sh_size).ok()?.min(max);
    data.get(start..start.checked_add(size)?)
}

fn find_section<'a>(elf: &'a Elf, name: &str) -> Option<&'a section_header::SectionHeader> {
    elf.section_headers
        .iter()
        .find(|s| elf.shdr_strtab.get_at(s.sh_name) == Some(name))
}

/// `.note.gnu.build-id` as hex.
fn elf_build_id(elf: &Elf, data: &[u8]) -> Option<String> {
    let note = section_bytes(data, find_section(elf, ".note.gnu.build-id")?, 256)?;
    let word = |at: usize| -> Option<usize> {
        let bytes: [u8; 4] = note.get(at..at + 4)?.try_into().ok()?;
        let value = if elf.little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        };
        usize::try_from(value).ok()
    };
    let name_len = word(0)?;
    let desc_len = word(4)?;
    let start = 12usize.checked_add(name_len.checked_next_multiple_of(4)?)?;
    let desc = note.get(start..start.checked_add(desc_len)?)?;
    if desc.is_empty() || desc.len() > 64 {
        return None;
    }
    Some(desc.iter().map(|b| format!("{b:02x}")).collect())
}

/// Distinct strings of the `.comment` section (compiler and linker versions).
fn elf_comment(elf: &Elf, data: &[u8]) -> Vec<String> {
    let Some(bytes) = find_section(elf, ".comment").and_then(|s| section_bytes(data, s, 64 * 1024))
    else {
        return Vec::new();
    };
    let mut seen: Vec<String> = Vec::new();
    for part in bytes.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let text = String::from_utf8_lossy(part).trim().to_string();
        if !text.is_empty() && !seen.contains(&text) {
            seen.push(text);
        }
    }
    seen
}

fn elf_summary(elf: &Elf, data: &[u8]) -> Summary {
    let bits = if elf.is_64 { 64 } else { 32 };
    let endian = if elf.little_endian { "LSB" } else { "MSB" };
    let kind = elf_kind(elf);
    let machine = elf_machine(elf.header.e_machine);
    let runnable = matches!(elf.header.e_type, header::ET_EXEC | header::ET_DYN);
    let mut rows: Rows = vec![
        ("format", format!("ELF {bits}-bit {endian}")),
        ("type", kind.to_string()),
        ("architecture", machine.clone()),
    ];
    if let Some(abi) = elf_os_abi(elf.header.e_ident[header::EI_OSABI]) {
        rows.push(("os abi", abi.to_string()));
    }
    if runnable && elf.entry != 0 {
        rows.push(("entry point", format!("{:#x}", elf.entry)));
    }
    if let Some(interp) = elf.interpreter {
        rows.push(("interpreter", interp.to_string()));
    }
    if runnable {
        let dynamic = elf.dynamic.is_some()
            || elf.interpreter.is_some()
            || elf
                .program_headers
                .iter()
                .any(|p| p.p_type == program_header::PT_DYNAMIC);
        rows.push((
            "linking",
            if dynamic { "dynamic" } else { "static" }.to_string(),
        ));
    }
    if !elf.libraries.is_empty() {
        rows.push(("needed", list(elf.libraries.iter(), MAX_LIST)));
    }
    if let Some(soname) = elf.soname {
        rows.push(("soname", soname.to_string()));
    }
    if !elf.rpaths.is_empty() {
        rows.push(("rpath", list(elf.rpaths.iter(), MAX_LIST)));
    }
    if !elf.runpaths.is_empty() {
        rows.push(("runpath", list(elf.runpaths.iter(), MAX_LIST)));
    }

    if elf.header.e_type != header::ET_CORE {
        let has_symtab = elf
            .section_headers
            .iter()
            .any(|s| s.sh_type == section_header::SHT_SYMTAB);
        rows.push(("stripped", yes_no(!has_symtab)));
    }
    if elf.dynsyms.len() > 1 {
        let (mut exported, mut imported) = (0usize, 0usize);
        for sym in elf.dynsyms.iter().skip(1) {
            if sym.st_shndx == 0 {
                imported += 1;
            } else if sym.st_bind() != goblin::elf::sym::STB_LOCAL {
                exported += 1;
            }
        }
        rows.push((
            "dynamic symbols",
            format!("{exported} exported, {imported} imported"),
        ));
    }

    let sections = elf.section_headers.len();
    if sections > 0 {
        let mut sized: Vec<(&str, u64)> = elf
            .section_headers
            .iter()
            .filter(|s| s.sh_type != section_header::SHT_NULL)
            .map(|s| (section_name(elf, s), s.sh_size))
            .collect();
        sized.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        let top: Vec<String> = sized
            .iter()
            .take(TOP_SECTIONS)
            .map(|(name, size)| format!("{name} {}", human_size(*size)))
            .collect();
        rows.push(("sections", format!("{sections}")));
        rows.push(("largest", top.join(", ")));
    }

    if runnable {
        rows.push(("security", elf_security(elf)));
    }

    if let Some(id) = elf_build_id(elf, data) {
        rows.push(("build id", id));
    }
    if find_section(elf, ".go.buildinfo").is_some()
        || find_section(elf, ".note.go.buildid").is_some()
    {
        rows.push(("built with", "Go".to_string()));
    }
    let comments = elf_comment(elf, data);
    if !comments.is_empty() {
        rows.push(("compiler", list(comments.iter(), 4)));
    }

    Summary {
        title: format!("{kind}, {machine}"),
        rows,
    }
}

/// NX, PIE, RELRO, canary and FORTIFY as one line.
fn elf_security(elf: &Elf) -> String {
    let stack = elf
        .program_headers
        .iter()
        .find(|p| p.p_type == program_header::PT_GNU_STACK);
    let nx = stack.is_some_and(|p| p.p_flags & program_header::PF_X == 0);
    let pie = match elf_kind(elf) {
        "PIE executable" => "yes",
        "executable" => "no",
        _ => "n/a",
    };
    let relro_segment = elf
        .program_headers
        .iter()
        .any(|p| p.p_type == program_header::PT_GNU_RELRO);
    let bind_now = elf.dynamic.as_ref().is_some_and(|d| {
        d.info.flags & dynamic::DF_BIND_NOW != 0
            || d.info.flags_1 & dynamic::DF_1_NOW != 0
            || d.dyns.iter().any(|e| e.d_tag == dynamic::DT_BIND_NOW)
    });
    let relro = match (relro_segment, bind_now) {
        (true, true) => "full",
        (true, false) => "partial",
        _ => "no",
    };
    let (mut canary, mut fortify) = (false, false);
    // Imported symbols: undefined entries of the dynamic table, or of the symbol table
    // for static executables and objects.
    let dynamic_names = elf
        .dynsyms
        .iter()
        .take(MAX_SYMBOLS)
        .filter(|s| s.st_shndx == 0)
        .filter_map(|s| elf.dynstrtab.get_at(s.st_name));
    let static_names = elf
        .syms
        .iter()
        .take(MAX_SYMBOLS)
        .filter(|s| s.st_shndx == 0)
        .filter_map(|s| elf.strtab.get_at(s.st_name));
    for name in dynamic_names.chain(static_names) {
        if name == "__stack_chk_fail" || name == "__stack_chk_guard" {
            canary = true;
        } else if name.starts_with("__") && name.ends_with("_chk") {
            fortify = true;
        }
    }
    format!(
        "NX {}, PIE {pie}, RELRO {relro}, canary {}, FORTIFY {}",
        yes_no(nx),
        yes_no(canary),
        yes_no(fortify)
    )
}

// ---------------------------------------------------------------- PE

fn pe_machine(machine: u16) -> String {
    match machine {
        0x014c => "x86 (i386)".to_string(),
        0x8664 => "x86-64".to_string(),
        0x01c0 | 0x01c4 => "ARM".to_string(),
        0xaa64 => "ARM64".to_string(),
        0x0200 => "Itanium".to_string(),
        0x5064 => "RISC-V 64".to_string(),
        other => format!("machine {other:#06x}"),
    }
}

fn pe_subsystem(subsystem: u16) -> String {
    match subsystem {
        1 => "native".to_string(),
        2 => "Windows GUI".to_string(),
        3 => "Windows console".to_string(),
        5 => "OS/2 console".to_string(),
        7 => "POSIX console".to_string(),
        9 => "Windows CE".to_string(),
        10 => "EFI application".to_string(),
        11 => "EFI boot service driver".to_string(),
        12 => "EFI runtime driver".to_string(),
        14 => "Xbox".to_string(),
        16 => "Windows boot application".to_string(),
        other => format!("subsystem {other}"),
    }
}

fn pe_summary(pe: &PE) -> Summary {
    let coff = &pe.header.coff_header;
    let is_dll = pe.is_lib || coff.characteristics & 0x2000 != 0;
    let kind = if is_dll { "DLL" } else { "EXE" };
    let machine = pe_machine(coff.machine);
    let mut rows: Rows = vec![
        (
            "format",
            if pe.is_64 { "PE32+" } else { "PE32" }.to_string(),
        ),
        ("type", kind.to_string()),
        ("architecture", machine.clone()),
    ];
    if let Some(optional) = pe.header.optional_header {
        rows.push(("subsystem", pe_subsystem(optional.windows_fields.subsystem)));
        if pe.entry != 0 {
            rows.push(("entry point", format!("{:#x}", pe.entry)));
        }
        let managed = optional
            .data_directories
            .get_clr_runtime_header()
            .is_some_and(|d| d.virtual_address != 0);
        if managed {
            rows.push((".net", "managed (CLR) assembly".to_string()));
        }
    }
    if coff.time_date_stamp != 0 {
        rows.push((
            "timestamp",
            format_timestamp(u64::from(coff.time_date_stamp)),
        ));
    }
    if !pe.libraries.is_empty() {
        rows.push(("imports", list(pe.libraries.iter(), MAX_LIST)));
    }
    if let Some(name) = pe.name {
        rows.push(("dll name", name.to_string()));
    }
    rows.push(("exports", format!("{}", pe.exports.len())));
    rows.push(("sections", format!("{}", pe.sections.len())));
    Summary {
        title: format!("{kind}, {machine}"),
        rows,
    }
}

// ---------------------------------------------------------------- Mach-O

fn macho_cpu(cpu: u32) -> String {
    match cpu {
        cputype::CPU_TYPE_X86_64 => "x86-64".to_string(),
        cputype::CPU_TYPE_X86 => "x86 (i386)".to_string(),
        cputype::CPU_TYPE_ARM64 => "ARM64".to_string(),
        cputype::CPU_TYPE_ARM64_32 => "ARM64_32".to_string(),
        cputype::CPU_TYPE_ARM => "ARM".to_string(),
        cputype::CPU_TYPE_POWERPC => "PowerPC".to_string(),
        cputype::CPU_TYPE_POWERPC64 => "PowerPC 64".to_string(),
        other => format!("cpu {other:#x}"),
    }
}

fn macho_filetype(filetype: u32) -> &'static str {
    match filetype {
        mach_header::MH_OBJECT => "relocatable object",
        mach_header::MH_EXECUTE => "executable",
        mach_header::MH_CORE => "core dump",
        mach_header::MH_DYLIB => "dynamic library",
        mach_header::MH_DYLINKER => "dynamic linker",
        mach_header::MH_BUNDLE => "bundle",
        mach_header::MH_DSYM => "debug symbols (dSYM)",
        mach_header::MH_KEXT_BUNDLE => "kernel extension",
        _ => "unknown type",
    }
}

fn macho_rows(macho: &MachO) -> Rows {
    let mut rows: Rows = vec![
        (
            "format",
            format!("Mach-O {}-bit", if macho.is_64 { 64 } else { 32 }),
        ),
        ("type", macho_filetype(macho.header.filetype).to_string()),
        ("architecture", macho_cpu(macho.header.cputype)),
    ];
    if macho.entry != 0 {
        rows.push(("entry point", format!("{:#x}", macho.entry)));
    }
    if let Some(name) = macho.name {
        rows.push(("install name", name.to_string()));
    }
    let libs: Vec<&str> = macho
        .libs
        .iter()
        .copied()
        .filter(|l| *l != "self")
        .collect();
    if !libs.is_empty() {
        rows.push(("dylibs", list(libs, MAX_LIST)));
    }
    if !macho.rpaths.is_empty() {
        rows.push(("rpath", list(macho.rpaths.iter(), MAX_LIST)));
    }
    rows.push(("segments", format!("{}", macho.segments.len())));
    rows
}

fn macho_summary(macho: &MachO) -> Summary {
    Summary {
        title: format!(
            "{}, {}",
            macho_filetype(macho.header.filetype),
            macho_cpu(macho.header.cputype)
        ),
        rows: macho_rows(macho),
    }
}

fn fat_summary(multi: &goblin::mach::MultiArch) -> Option<Summary> {
    let arches = multi.arches().ok()?;
    if arches.is_empty() || arches.len() as u32 > MAX_FAT_SLICES {
        return None;
    }
    let slices: Vec<String> = arches
        .iter()
        .map(|a| {
            format!(
                "{} ({})",
                macho_cpu(a.cputype),
                human_size(u64::from(a.size))
            )
        })
        .collect();
    let mut rows: Rows = vec![
        ("format", "Mach-O universal (fat)".to_string()),
        ("slices", list(slices, MAX_LIST)),
    ];
    // Details of the first slice that parses.
    for index in 0..arches.len() {
        if let Ok(goblin::mach::SingleArch::MachO(macho)) = multi.get(index) {
            let mut inner = macho_rows(&macho);
            inner.retain(|(key, _)| *key != "format");
            rows.extend(inner);
            break;
        }
    }
    Some(Summary {
        title: format!("universal binary, {} slices", arches.len()),
        rows,
    })
}

// ---------------------------------------------------------------- archive

fn archive_summary(archive: &Archive, data: &[u8]) -> Option<Summary> {
    let names: Vec<&str> = archive.members();
    // A .deb is an ar archive too; it is not a library.
    if names.first() == Some(&"debian-binary") {
        return None;
    }
    let mut rows: Rows = vec![
        ("format", "ar archive".to_string()),
        ("members", format!("{}", names.len())),
    ];
    // The architecture of the first object file inside.
    for name in names.iter().take(64) {
        if let Ok(bytes) = archive.extract(name, data)
            && bytes.starts_with(b"\x7fELF")
            && let Ok(Object::Elf(elf)) = Object::parse(bytes)
        {
            rows.push(("architecture", elf_machine(elf.header.e_machine)));
            break;
        }
    }
    let shown: Vec<&str> = names.iter().copied().filter(|n| !n.is_empty()).collect();
    if !shown.is_empty() {
        rows.push(("objects", list(shown, MAX_LIST)));
    }
    Some(Summary {
        title: format!("static library, {} members", names.len()),
        rows,
    })
}

// ---------------------------------------------------------------- WebAssembly

fn wasm(data: &[u8]) -> Option<Summary> {
    use wasmparser::{Encoding, KnownCustom, Parser, Payload};

    let mut version = None;
    let mut component = false;
    let mut depth = 0u32;
    let mut import_count = 0usize;
    let mut import_names: Vec<String> = Vec::new();
    let mut export_count = 0usize;
    let mut export_names: Vec<String> = Vec::new();
    let mut memory: Vec<String> = Vec::new();
    let mut functions = 0u32;
    let mut customs: Vec<String> = Vec::new();
    let mut producers: Vec<String> = Vec::new();
    let mut start = false;

    for payload in Parser::new(0).parse_all(data) {
        let payload = payload.ok()?;
        match payload {
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) => depth = depth.saturating_sub(1),
            // Nested modules and components are not summarized.
            _ if depth > 0 => {}
            Payload::Version { num, encoding, .. } => {
                version = Some(num);
                component = encoding == Encoding::Component;
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.ok()?;
                    import_count += 1;
                    if import_names.len() < 5 {
                        import_names.push(format!("{}.{}", import.module, import.name));
                    }
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.ok()?;
                    export_count += 1;
                    if export_names.len() < 5 {
                        export_names.push(export.name.to_string());
                    }
                }
            }
            Payload::FunctionSection(reader) => functions += reader.count(),
            Payload::StartSection { .. } => start = true,
            Payload::MemorySection(reader) => {
                for mem in reader.into_iter().take(MAX_LIST) {
                    let mem = mem.ok()?;
                    let mut text = format!("{} pages min", mem.initial);
                    match mem.maximum {
                        Some(max) => {
                            let _ = write!(text, ", {max} max");
                        }
                        None => text.push_str(", no max"),
                    }
                    if mem.shared {
                        text.push_str(", shared");
                    }
                    if mem.memory64 {
                        text.push_str(", 64-bit");
                    }
                    memory.push(text);
                }
            }
            Payload::CustomSection(custom) => {
                if customs.len() < MAX_LIST {
                    customs.push(custom.name().to_string());
                }
                if let KnownCustom::Producers(reader) = custom.as_known() {
                    for field in reader.into_iter().take(8) {
                        let field = field.ok()?;
                        let values: Vec<String> = field
                            .values
                            .into_iter()
                            .take(4)
                            .filter_map(Result::ok)
                            .map(|v| {
                                if v.version.is_empty() {
                                    v.name.to_string()
                                } else {
                                    format!("{} {}", v.name, v.version)
                                }
                            })
                            .collect();
                        producers.push(format!("{}: {}", field.name, values.join(", ")));
                    }
                }
            }
            _ => {}
        }
    }
    let version = version?;
    let kind = if component { "component" } else { "module" };
    let mut rows: Rows = vec![
        ("format", "WebAssembly".to_string()),
        ("type", kind.to_string()),
        ("version", format!("{version}")),
    ];
    if !component {
        rows.push(("functions", format!("{functions}")));
        rows.push(("imports", format!("{import_count}")));
        if !import_names.is_empty() {
            rows.push(("import names", list_more(&import_names, import_count)));
        }
        rows.push(("exports", format!("{export_count}")));
        if !export_names.is_empty() {
            rows.push(("export names", list_more(&export_names, export_count)));
        }
        if !memory.is_empty() {
            rows.push(("memory", memory.join("; ")));
        }
        if start {
            rows.push(("start", "has a start function".to_string()));
        }
    }
    if !customs.is_empty() {
        rows.push(("custom sections", list(customs.iter(), MAX_LIST)));
    }
    if !producers.is_empty() {
        rows.push(("producers", producers.join("; ")));
    }
    Some(Summary {
        title: format!("WebAssembly {kind}"),
        rows,
    })
}

/// The shown names followed by "… N more" when the total is larger.
fn list_more(shown: &[String], total: usize) -> String {
    let mut text = shown.join(", ");
    if total > shown.len() {
        let _ = write!(text, ", … {} more", total - shown.len());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module importing `env.log`, exporting `run` and `memory`, with 1 page of memory.
    fn tiny_wasm() -> Vec<u8> {
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        // type section: one func type () -> ()
        m.extend([1, 4, 1, 0x60, 0, 0]);
        // import section: env.log, func of type 0
        m.extend([2, 11, 1, 3, b'e', b'n', b'v', 3, b'l', b'o', b'g', 0, 0]);
        // function section: one function of type 0 (index 1 overall)
        m.extend([3, 2, 1, 0]);
        // memory section: min 1, max 4
        m.extend([5, 4, 1, 1, 1, 4]);
        // export section: "run" func 1, "memory" memory 0
        m.extend([
            7, 16, 2, 3, b'r', b'u', b'n', 0, 1, 6, b'm', b'e', b'm', b'o', b'r', b'y', 2, 0,
        ]);
        // code section: one empty body
        m.extend([10, 4, 1, 2, 0, 0x0b]);
        m
    }

    fn row<'a>(summary: &'a Summary, key: &str) -> Option<&'a str> {
        summary
            .rows
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn list_caps_entries() {
        assert_eq!(list(["a", "b"], 3), "a, b");
        assert_eq!(list(["a", "b", "c", "d"], 2), "a, b, … 2 more");
        assert_eq!(list(Vec::<String>::new(), 2), "");
    }

    #[test]
    fn formats_timestamps() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14 22:13:20 UTC");
        assert_eq!(format_timestamp(951_782_400), "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn recognizes_magics() {
        assert!(wants(b"\x7fELF\x02\x01"));
        assert!(wants(b"\0asm\x01\0\0\0"));
        assert!(wants(b"!<arch>\nfoo"));
        assert!(wants(b"MZ\x90\0"));
        assert!(wants(&[0xcf, 0xfa, 0xed, 0xfe]));
        assert!(!wants(b"hello"));
        assert!(!wants(b""));
    }

    #[test]
    fn summarizes_wasm() {
        let summary = summarize(&tiny_wasm()).expect("valid module");
        assert_eq!(summary.title, "WebAssembly module");
        assert_eq!(row(&summary, "version"), Some("1"));
        assert_eq!(row(&summary, "imports"), Some("1"));
        assert_eq!(row(&summary, "import names"), Some("env.log"));
        assert_eq!(row(&summary, "exports"), Some("2"));
        assert_eq!(row(&summary, "export names"), Some("run, memory"));
        assert_eq!(row(&summary, "memory"), Some("1 pages min, 4 max"));
        assert_eq!(row(&summary, "functions"), Some("1"));
    }

    #[test]
    fn broken_input_gives_none() {
        assert!(summarize(b"\0asm\x01\0\0\0\x01\xff").is_none());
        assert!(summarize(b"\x7fELF\x02\x01\x01\0").is_none());
        assert!(summarize(b"MZ not a pe file").is_none());
        assert!(summarize(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0x34]).is_none());
        assert!(summarize(b"random bytes").is_none());
    }
}
