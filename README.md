# zaf-mpp

A pure-Rust, read-only parser for Microsoft Project MPP14 files (Project
2010 through 365). It is a deliberate port of the read path of
[MPXJ](https://github.com/joniles/mpxj), Jon Iles's Java library for
reading and writing project file formats.

## Scope

MPP14 only: Project 2010, 2013, 2016, 2019, and 365 all save to this
format. Opening an MPP8, MPP9, or MPP12 file (Project 98 through 2007)
returns `MppError::UnsupportedVersion` naming the format detected.

In scope: project properties, task hierarchy and scheduling fields
(dates, duration, work, percent complete, milestones, constraints, manual
vs auto scheduling, critical path and slack), task dependencies with lag,
resources, resource assignments, baselines 0 through 10, and calendar
working time including exceptions.

Out of scope: all write support, enterprise and custom fields, views and
other presentation data, and VBA.

## Usage

```rust
let project = zaf_mpp::read_mpp("plan.mpp")?;

for task in &project.tasks {
    println!("{}: {:?}", task.id, task.name);
}
```

`read_mpp_bytes` is also available for callers that already have the file
in memory. Both return `Result<Project, MppError>`.

A C ABI is provided for applications that link the `cdylib` build target
dynamically; see the rustdoc on `ffi::zaf_mpp_parse`.

## Install

Add it as a git dependency:

```toml
[dependencies]
zaf-mpp = { git = "https://github.com/zenith-consulting-services/zaf-mpp" }
```

## Licence

zaf-mpp is a derivative work of MPXJ. All knowledge of the MPP14 binary
format encoded in this crate, including block layouts, field maps, and
format-specific quirks, comes from the MPXJ source. Credit for that work
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
