# smartread

Reads the S.M.A.R.T. health data of disks **in Rust, without running `smartctl`**:
identity, the ATA attribute table with thresholds, the drive's own health
verdict, the result of the last self test, and the NVMe SMART / Health log.

```rust
let report = smartread::read("/dev/sda")?;          // Linux
// let report = smartread::read(r"\\.\PhysicalDrive0")?;   // Windows
println!("{} {:?}, {:?} °C", report.model, report.health, report.temperature);
for a in &report.attributes {
    println!("{:>3} {:?} value {} worst {} threshold {:?} raw {}", a.id, a.name(), a.value, a.worst, a.threshold, a.raw);
}
# Ok::<(), smartread::Error>(())
```

`cargo run --example smartread -- /dev/sda` prints the same as a table.

## What it reads

| Drive | Commands |
|---|---|
| **ATA / SATA** (also SATA disks behind a SAT-capable USB bridge) | `IDENTIFY DEVICE`, `SMART READ DATA`, `SMART READ THRESHOLDS`, `SMART RETURN STATUS` through ATA PASS-THROUGH(16) |
| **NVMe** | *Identify Controller* and *Get Log Page* 0x02 (SMART / Health Information) |
| **SCSI / SAS** | not implemented |

Everything is **read only**: nothing is written to the drive, no self test is
started, no SMART setting is changed.

## Platforms and what has been verified

| Platform | ATA | NVMe | State |
|---|---|---|---|
| **Linux** (`SG_IO`, `NVME_IOCTL_ADMIN_CMD`) | yes | yes | ATA verified on two WD Red 2 TB SATA disks on a 32-bit (`i686-unknown-linux-musl`) machine: every attribute, threshold, self test status and the health verdict equal the output of `smartctl 7.5`. NVMe read on a Samsung SSD 990 PRO 2 TB (identity, temperature, spare, endurance, counters, plausible); a comparison with `smartctl` output is still to be made. |
| **Windows** (`IOCTL_ATA_PASS_THROUGH`, `IOCTL_STORAGE_QUERY_PROPERTY`) | yes | yes | compiles and runs; opening the drive and the error path were checked on a Windows 11 virtual machine, whose VirtIO disks refuse both commands (`ERROR_INVALID_FUNCTION`). Reading real SMART data on Windows is **not yet verified**. |
| **macOS** | – | – | **not implemented yet** (the plan is `IOKit`: `IOATASMARTInterface` and the NVMe SMART user client). `read` returns `Error::Unsupported`. |

Please report results from real hardware (especially NVMe, USB bridges,
Windows, SAS) to <contact@onesimus.io>.

## Permissions

The kernel only lets privileged callers send these commands, the same as for
`smartctl`:

* Linux ATA: root or `CAP_SYS_RAWIO`, and permission to open the device
  (usually group `disk`). NVMe: root or `CAP_SYS_ADMIN` on `/dev/nvme0`.
* Windows: administrator, on `\\.\PhysicalDriveN`.

A daemon that must not run as root can keep the capability as an *ambient*
capability (systemd `AmbientCapabilities=CAP_SYS_RAWIO`, OpenRC
`capabilities="^cap_sys_rawio"`) and join the `disk` group.

## Design

* **No C library, no `smartctl`.** The Linux code uses `libc::ioctl` with
  `repr(C)` copies of `struct sg_io_hdr` and `struct nvme_admin_cmd`; the
  Windows code uses `windows-sys`. `unsafe` appears only at the ioctl calls.
* **The parsers take bytes.** `ata::parse_identify`, `ata::parse_smart_data`,
  `ata::parse_thresholds` and `nvme::parse_health_log` work on the raw
  512-byte sectors and run (and are tested) on every platform; only the
  device access depends on the operating system.
* **ATA health verdict.** `SMART RETURN STATUS` is a non-data command whose
  answer is in the registers; the crate asks for them in the SAT sense data
  (`CK_COND`). LBA mid/high `0x4F`/`0xC2` is *passed*, `0xF4`/`0x2C` is
  *failed*, anything else (a bridge that does not return the registers) is
  `Health::Unknown`.
* **What the crate does not decide.** It reports numbers and the drive's own
  verdict. Whether a disk with 232 pending sectors is "warning" or "critical"
  is a policy for the caller; `Attribute::failing()` only says that a value
  has reached its threshold.

## Limits

* SAT only. USB bridges that need the 12-byte variant or a vendor command
  (some Cypress, JMicron, Prolific chips) are not tried.
* NVMe over a USB bridge, drives behind a hardware RAID controller and
  `dm`/`md` devices are not reachable.
* Raw values are the six bytes as a number. Vendor specific encodings (for
  example the temperature field with min/max, or hours in minutes) are not
  decoded beyond temperature (194/190) and power-on hours (9).
* The attribute names are the common vendor independent ones.

## Tests

`cargo test -p smartread` runs the parser tests with sectors built from the
byte layout of real drives and checks the ioctl numbers; on a machine with a
SATA disk and root, `cargo run --example smartread -- /dev/sdX` can be
compared with `smartctl -A -H`.

## Licence and status

Version 0.1.0, not yet published on crates.io (`publish = false` until the
licence of the release is decided). Contact: <contact@onesimus.io>.
