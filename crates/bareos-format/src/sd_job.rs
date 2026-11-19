//! Job-Kommando des Directors an den Storage Daemon.
//!
//! Zweck: Bevor ein Job Daten bewegt, meldet der Director ihn beim SD an:
//! eine Zeile `JobId=… job=… job_name=… client_name=… type=… level=…
//! FileSet=… …`. Der SD antwortet mit `3000 OK Job SDid=… SDtime=…
//! Authorization=…`. Dieses Modul erzeugt die Zeile; es gibt sie im
//! Repository nur noch hier.
//!
//! Regeln (wie `msgchan.cc`):
//!
//! - `job_name`, `client_name` und `FileSet` gehen durch
//!   [`crate::bash::bash_spaces`]; der eindeutige Jobname (`job=`) nicht.
//! - Fehlt der Client oder das FileSet, steht `**None**`.
//! - Fehlt die FileSet-Prüfsumme, steht `**Dummy**`.
//! - `type` und `level` sind die Buchstaben des Jobtyps bzw. der Stufe als
//!   Dezimalzahl (`'B'` = 66, `'F'` = 70).
//!
//! Datenfluss: odir (`sd_copy`, `backup_worker`, `verify`, `job_worker` über
//! `osd::job`) → SD.
//!
//! Bareos-Gegenstück: `core/src/dird/msgchan.cc` `StartStorageDaemonJob()`
//! (Formatzeichenkette `jobcmd`), Gegenseite `core/src/stored/job.cc`
//! `job_cmd()`.
//!
//! Zusammengeführt (19.11.2026) aus drei Fassungen: `odir/src/sd_copy.rs`
//! `format_job_command` (Vorlage, unverändert), die Zeile in
//! `odir/src/verify.rs` `start_sd` (gleich bis auf `**None**` bei leerem
//! Client/FileSet) und `osd/src/job.rs` `sd_job_setup`. Die osd-Fassung war
//! fehlerhaft (Defekt D11): Sie schrieb die Client-**Adresse** als
//! `client_name`, ein festes `FileSet=LinuxAll`, keine BashSpaces, beim
//! Backup `FileSetMD5=0` und `BackupFormat=NativeBackup`. Sie nutzt jetzt
//! [`JobCommand`] mit dem Client-Namen.

use crate::bash::bash_spaces;

/// One Job command of the Director to the Storage Daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobCommand<'a> {
    /// `JobId=`: the job's id in the catalog.
    pub job_id: u32,
    /// `job=`: the unique job name (`<Job>.<date>_<time>_<n>`), sent as is.
    pub unique_job: &'a str,
    /// `job_name=`: name of the Job resource (bashed).
    pub job_name: &'a str,
    /// `client_name=`: name of the Client resource (bashed, `**None**` if empty).
    pub client_name: &'a str,
    /// `type=`: job type letter (`B` backup, `R` restore, `V` verify, `c` copy …).
    pub job_type: char,
    /// `level=`: level letter (`F`, `I`, `D`, `O` …; `' '` for none).
    pub level: char,
    /// `FileSet=`: name of the FileSet (bashed, `**None**` if empty).
    pub fileset: &'a str,
    /// `FileSetMD5=`: FileSet digest (`**Dummy**` if empty).
    pub fileset_md5: &'a str,
    /// `NoAttr=`: the SD does not send attributes back.
    pub no_attributes: bool,
    /// `SpoolAttr=`: attributes are spooled.
    pub spool_attributes: bool,
    /// `SpoolData=`: data is spooled.
    pub spool_data: bool,
    /// `PreferMountedVols=`: prefer volumes already mounted.
    pub prefer_mounted_volumes: bool,
    /// `SpoolSize=`: spool size limit in bytes.
    pub spool_size: i64,
}

impl JobCommand<'_> {
    /// The command line, ending in `\n`.
    pub fn format(&self) -> String {
        let or_none = |v: &str| if v.is_empty() { "**None**".to_string() } else { bash_spaces(v) };
        format!(
            "JobId={} job={} job_name={} client_name={} type={} level={} FileSet={} NoAttr={} SpoolAttr={} \
             FileSetMD5={} SpoolData={} PreferMountedVols={} SpoolSize={} rerunning=0 VolSessionId=0 \
             VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n",
            self.job_id,
            self.unique_job,
            bash_spaces(self.job_name),
            or_none(self.client_name),
            self.job_type as u32,
            self.level as u32,
            or_none(self.fileset),
            self.no_attributes as i32,
            self.spool_attributes as i32,
            if self.fileset_md5.is_empty() { "**Dummy**" } else { self.fileset_md5 },
            self.spool_data as i32,
            self.prefer_mounted_volumes as i32,
            self.spool_size
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy job command as `odir/src/sd_copy.rs` built it.
    fn copy_command() -> JobCommand<'static> {
        JobCommand {
            job_id: 632,
            unique_job: "mig-copy.2026-10-17_06.53.05_05",
            job_name: "mig copy",
            client_name: "client one",
            job_type: 'c',
            level: 'F',
            fileset: "Full Set",
            fileset_md5: "",
            no_attributes: false,
            spool_attributes: true,
            spool_data: false,
            prefer_mounted_volumes: true,
            spool_size: 0,
        }
    }

    /// odir/src/sd_copy.rs `format_job_command`, copied unchanged (19.11.2026),
    /// with its two structs reduced to the fields it reads.
    #[allow(clippy::too_many_arguments)]
    fn old_sd_copy(job_id: u32, unique_job: &str, job_resource: &str, client: &str, job_type: char, level: char,
        fileset: &str, fileset_md5: &str, no_attributes: bool, spool_attributes: bool, spool_data: bool,
        prefer_mounted_volumes: bool, spool_size: i64) -> String {
        let bash = crate::bash::bash_spaces;
        let or_none = |v: &str| if v.is_empty() { "**None**".to_string() } else { bash(v) };
        format!("JobId={} job={} job_name={} client_name={} type={} level={} FileSet={} NoAttr={} SpoolAttr={} \
                 FileSetMD5={} SpoolData={} PreferMountedVols={} SpoolSize={} rerunning=0 VolSessionId=0 \
                 VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n",
            job_id, unique_job, bash(job_resource), or_none(client), job_type as u32,
            level as u32, or_none(fileset), no_attributes as i32, spool_attributes as i32,
            if fileset_md5.is_empty() { "**Dummy**" } else { fileset_md5 }, spool_data as i32,
            prefer_mounted_volumes as i32, spool_size)
    }

    #[test]
    fn matches_the_sd_copy_formatter() {
        for (client, fileset, md5, size) in [("client one", "Full Set", "", 0i64), ("", "", "abc", 4096), ("c", "f", "m", 1)] {
            let mut cmd = copy_command();
            cmd.client_name = client;
            cmd.fileset = fileset;
            cmd.fileset_md5 = md5;
            cmd.spool_size = size;
            assert_eq!(
                cmd.format(),
                old_sd_copy(632, "mig-copy.2026-10-17_06.53.05_05", "mig copy", client, 'c', 'F', fileset, md5, false,
                    true, false, true, size)
            );
        }
    }

    #[test]
    fn copy_golden() {
        assert_eq!(
            copy_command().format(),
            "JobId=632 job=mig-copy.2026-10-17_06.53.05_05 job_name=mig\u{1}copy client_name=client\u{1}one type=99 \
             level=70 FileSet=Full\u{1}Set NoAttr=0 SpoolAttr=1 FileSetMD5=**Dummy** SpoolData=0 \
             PreferMountedVols=1 SpoolSize=0 rerunning=0 VolSessionId=0 VolSessionTime=0 Quota=0 Protocol=0 \
             BackupFormat=Native\n"
        );
    }

    #[test]
    fn matches_the_verify_line_for_named_client_and_fileset() {
        // odir/src/verify.rs start_sd, copied unchanged (19.11.2026)
        let bash = crate::bash::bash_spaces;
        let old = format!("JobId={} job={} job_name={} client_name={} type={} level={} FileSet={} NoAttr=0 SpoolAttr=0 \
        FileSetMD5=**Dummy** SpoolData=0 PreferMountedVols=1 SpoolSize=0 rerunning=0 VolSessionId=0 VolSessionTime=0 \
        Quota=0 Protocol=0 BackupFormat=Native\n", 7, "VVol.2026-11-19_05.06.26_06", bash("V Vol"), bash("my fd"),
            b'V', b'O', bash("V Set"));
        let new = JobCommand {
            job_id: 7,
            unique_job: "VVol.2026-11-19_05.06.26_06",
            job_name: "V Vol",
            client_name: "my fd",
            job_type: 'V',
            level: 'O',
            fileset: "V Set",
            fileset_md5: "",
            no_attributes: false,
            spool_attributes: false,
            spool_data: false,
            prefer_mounted_volumes: true,
            spool_size: 0,
        };
        assert_eq!(new.format(), old);
    }
}
