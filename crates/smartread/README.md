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
| **SCSI / SAS** (Linux) | `INQUIRY` (and the unit serial VPD page 0x80), `LOG SENSE` of the pages Informational Exceptions (0x2F: predicted failure, temperature), Temperature (0x0D), Self-Test Results (0x10) and the Write/Read/Verify Error Counters (0x02/0x03/0x05), `READ DEFECT DATA(10)` for the grown defect list |

Everything is **read only**: nothing is written to the drive, no self test is
started, no SMART setting is changed.

## Platforms and what has been verified

| Platform | ATA | NVMe | State |
|---|---|---|---|
| **Linux** (`SG_IO`, `NVME_IOCTL_ADMIN_CMD`) | yes | yes | ATA verified on two WD Red 2 TB SATA disks on a 32-bit (`i686-unknown-linux-musl`) machine: every attribute, threshold, self test status and the health verdict equal the output of `smartctl 7.5`. NVMe read on a Samsung SSD 990 PRO 2 TB (identity, temperature, spare, endurance, counters, plausible); a comparison with `smartctl` output is still to be made. |
| **Windows** (`IOCTL_ATA_PASS_THROUGH`, `IOCTL_STORAGE_QUERY_PROPERTY`) | yes | yes | compiles and runs; opening the drive and the error path were checked on a Windows 11 virtual machine, whose VirtIO disks refuse both commands (`ERROR_INVALID_FUNCTION`). Reading real SMART data on Windows is **not yet verified**. |
| **Linux SCSI / SAS** | – | – | the parsers follow SPC-4 and are tested on pages built from its byte layouts; the commands were checked against SATA disks only (they answer `INQUIRY` through SAT, not the log pages). **Not verified on a SAS drive** (no hardware); reports are welcome. |
| **macOS** | – | – | **not implemented yet** (the plan is `IOKit`: `IOATASMARTInterface` and the NVMe SMART user client). `read` returns `Error::Unsupported`. |

Please report results from real hardware (especially USB bridges, Windows,
SAS) to <contact@onesimus.io>.

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
  `dm`/`md` devices are not reachable. SAS behind a RAID controller needs the
  controller's own pass-through (not supported).
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

## What is still open, and why only hardware can close it

The crate is complete for what could be tested here: Linux ATA/SATA and NVMe
(read on real drives), the parsers on every platform, the SCSI/SAS log page
parsers against SPC-4 byte layouts, and the Windows code against the error
path. Three parts cannot be finished without the matching hardware; each has a
short procedure, and a report from it settles the row in the table above.

| Open | Needs | Procedure |
|---|---|---|
| **SAS drive** (Linux) | a SAS disk on an HBA (not behind a RAID controller) | `cargo run --example smartread -- /dev/sdX` as root; compare identity, temperature, grown defect count and the error counters with `smartctl -a /dev/sdX` (smartctl 7.x) |
| **Windows, real disk** | a physical machine (the VirtIO disks of a VM refuse the commands) | run the example as administrator against `\\.\PhysicalDrive0`; compare with `smartctl -a /dev/pd0` |
| **macOS** | a Mac | not implemented: `read` returns `Error::Unsupported`. The plan is IOKit (`IOATASMARTInterface` for SATA, the NVMe SMART user client for NVMe), whose plug-in vtables are only checkable on a Mac; guessing their layout without one could crash the caller, so it is left out on purpose |
| **NVMe against `smartctl`** | root on the machine with the drive | `sudo cargo run --example smartread -- /dev/nvme0` and `sudo smartctl -a /dev/nvme0`; temperature, spare, endurance and the counters must agree |

Send the output of both programs to <contact@onesimus.io>.

