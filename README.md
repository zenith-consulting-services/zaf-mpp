# zaf-mpp

A pure-Rust, read-only parser for project schedule files: Microsoft
Project MPP14 (Project 2010 through 365) and Primavera P6 exports in both
XER and PMXML form. It is a deliberate port of the read path of
[MPXJ](https://github.com/joniles/mpxj), Jon Iles's Java library for
reading and writing project file formats.

## Scope

Formats:

- **MPP14** — Project 2010, 2013, 2016, 2019, and 365 all save to this
  format. Opening an MPP8, MPP9, or MPP12 file (Project 98 through 2007)
  returns `MppError::UnsupportedVersion` naming the format detected.
- **Primavera P6 XER** — the tab-delimited export produced by every P6
  version.
- **Primavera P6 PMXML** — the `APIBusinessObjects` XML export.

Every format parses into the same `Project` model, so consumers are
format-agnostic. A P6 export containing several projects yields the
exported one (cross-project relations are dropped).

In scope: project properties, task hierarchy and scheduling fields
(dates, duration, work, percent complete, milestones, constraints, manual
vs auto scheduling, critical path and slack), task dependencies with lag,
resources, resource assignments, baselines 0 through 10 (for P6, baseline
0 is populated from the planned/"target" values, matching MPXJ's
planned-attributes baseline strategy), and calendar working time
including exceptions.

Out of scope: all write support, enterprise and custom fields, views and
other presentation data, VBA, and P6 concepts with no MPP counterpart in
the model (activity codes, UDFs, expense items, activity steps, roles,
shifts, notebook topics).

## Usage

```rust
// Auto-detects MPP, XER, or PMXML from the file content:
let project = zaf_mpp::read_project("plan.mpp")?;

for task in &project.tasks {
    println!("{}: {:?}", task.id, task.name);
}
```

Per-format entry points (`read_mpp`, `read_xer`, `read_pmxml`) and
`*_bytes` variants for in-memory buffers are also available. All return
`Result<Project, MppError>`.

A C ABI is provided for applications that link the `cdylib` build target
dynamically; see the rustdoc on `ffi::zaf_mpp_parse` (MPP-only, kept for
ABI stability) and `ffi::zaf_mpp_parse_project` (format auto-detecting).

## Install

Add it as a git dependency:

```toml
[dependencies]
zaf-mpp = { git = "https://github.com/zenith-consulting-services/zaf-mpp" }
```

## Licence

zaf-mpp is a derivative work of MPXJ. All knowledge of the MPP14 binary
format and of the Primavera XER/PMXML formats encoded in this crate,
including block layouts, field maps, table mappings, and format-specific
quirks, comes from the MPXJ source. Credit for that work
belongs to Jon Iles and the MPXJ contributors.

This crate is licensed under the GNU Lesser General Public License,
version 2.1 or later (see `LICENSE`), the same terms as MPXJ itself. An
application that links zaf-mpp dynamically (the `cdylib` build target) is
not itself covered by the LGPL: it can be licensed under any terms. Such
an application must ship the LGPL licence text and must allow the user to
relink it against a modified version of zaf-mpp, per LGPL section 6.

## Credits

Format knowledge, field offsets, and block layouts in this crate are
ported from [MPXJ](https://github.com/joniles/mpxj) by Jon Iles and the
MPXJ contributors, originally published by Packwood Software.
