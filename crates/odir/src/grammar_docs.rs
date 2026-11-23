//! Man-page prose of the console grammar: the DESCRIPTION paragraphs and
//! examples of commands and subcommands, kept apart from the grammar table
//! ([`crate::grammar_table`]) so that the table stays a readable list of
//! words and arguments. [`apply`] adds them to a root node when the grammar
//! is built ([`crate::command_grammar::roots_for`]).
//!
//! A [`Doc`] names its node by path: the command word and the subcommand
//! words (`list jobs`, `status client`, `.acl set`). A node either has its
//! paragraphs in the grammar table or here, never both (test
//! `docs_name_existing_nodes`); examples from both places are shown.

#![warn(clippy::missing_docs_in_private_items)]

use crate::command_grammar::Node;

/// Prose of one node.
pub struct Doc {
    /// Command word and subcommand words, separated by one blank.
    pub path: &'static str,
    /// DESCRIPTION paragraphs (English, plain text).
    pub long: &'static [&'static str],
    /// Example lines with a one-line explanation.
    pub examples: &'static [(&'static str, &'static str)],
}

/// Add the prose of [`DOCS`] to `root` and its subcommands. A path whose
/// node does not exist is skipped (the test catches it).
pub fn apply(root: &mut Node) {
    for doc in DOCS {
        let mut words = doc.path.split(' ');
        if words.next() != Some(root.word.as_str()) {
            continue;
        }
        let mut node = &mut *root;
        let mut found = true;
        for word in words {
            match node.subs.iter_mut().find(|sub| sub.word == word) {
                Some(sub) => node = sub,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if !found {
            continue;
        }
        if node.long.is_empty() {
            node.long = doc.long.iter().map(|p| p.to_string()).collect();
        }
        node.examples.extend(doc.examples.iter().map(|(line, text)| (line.to_string(), text.to_string())));
    }
}

/// The prose, in grammar order.
pub static DOCS: &[Doc] = &[
    Doc {
        path: "version",
        long: &[],
        examples: &[
            ("version", "name, version and build date of the running Director"),
        ],
    },
    Doc {
        path: "time",
        long: &[],
        examples: &[
            ("time", "check the Director clock when schedules seem off by a time zone"),
        ],
    },
    Doc {
        path: "quit",
        long: &[],
        examples: &[
            ("quit", "close the console session"),
        ],
    },
    Doc {
        path: "help",
        long: &[],
        examples: &[
            ("help", "table of every command this console may run"),
            ("help run", "all arguments of run"),
        ],
    },
    Doc {
        path: "status",
        long: &[],
        examples: &[
            ("status", "ask whether to show the Director, a storage, a client, the scheduler or all"),
            ("status director days=3", "Director status with jobs scheduled for the next three days"),
        ],
    },
    Doc {
        path: "status all",
        long: &[
            "Prints the Director status, followed by the status report of every Storage Daemon and every File Daemon the console may see. A daemon reachable under the same address is queried only once, so several Storage resources pointing at one Storage Daemon produce a single report.",
        ],
        examples: &[
            ("status all", "Director, every storage and every client in one report"),
        ],
    },
    Doc {
        path: "status director",
        long: &[
            "Shows the Director's header line (version, start time, job counters, database type), the jobs scheduled for the coming days, the running jobs, the most recently terminated jobs and connections initiated by clients. Any word starting with dir selects this report, so status dir works as well.",
            "days= sets how many days ahead scheduled jobs are listed, from 0 to 500, default 1. An invalid value is ignored with a notice and 1 day is used.",
        ],
        examples: &[
            ("status director", "header, scheduled, running and terminated jobs"),
            ("status director days=7", "scheduled jobs of the coming week"),
        ],
    },
    Doc {
        path: "status client",
        long: &[
            "Connects to the File Daemon of the given client and passes on its status report: version, running jobs and recently terminated jobs. Without a value, or with a name the console may not see or that does not exist, the Director shows the list of clients to choose from.",
        ],
        examples: &[
            ("status client=bareos-fd", "what the File Daemon bareos-fd is doing right now"),
            ("status client", "choose the client from a list"),
        ],
    },
    Doc {
        path: "status storage",
        long: &[
            "Connects to the Storage Daemon of the given Storage resource and passes on its report: devices, mounted volumes, running and terminated jobs. Without a value or with an unknown name the Director offers a list of storages. sd= is accepted as a synonym for storage=.",
            "With slots the Director prints the slot table of an autochanger instead of the status report; for a device that is not an autochanger the Storage Daemon answers with an error.",
        ],
        examples: &[
            ("status storage=File", "devices and jobs of the Storage Daemon behind File"),
            ("status storage=Tape slots", "slot table of the autochanger"),
        ],
    },
    Doc {
        path: "status job",
        long: &[
            "Shows the Storage Daemon report for the storage used by the running job with this name. If no job of that name is running, the Director says so; for finished jobs use list jobs instead.",
        ],
        examples: &[
            ("status job=BackupClient1", "storage status of the running BackupClient1 job"),
        ],
    },
    Doc {
        path: "status jobid",
        long: &[
            "Like status job, but selects the running job by its JobId. If the job is not running, the Director reports \"JobId <n> is not running.\".",
        ],
        examples: &[
            ("status jobid=42", "storage status of the running job 42"),
        ],
    },
    Doc {
        path: "status ujobid",
        long: &[
            "Like status job, but selects the running job by its unique job name, the name with date, time and sequence number that the Director gives every job run.",
        ],
        examples: &[
            ("status ujobid=BackupClient1.2026-11-20_21.00.00_09", "storage status of exactly this job run"),
        ],
    },
    Doc {
        path: "status scheduler",
        long: &[
            "Lists which schedules trigger which jobs and previews the upcoming runs with level and pool. schedule= restricts the output to one schedule, client= to the jobs of one client and job= to one job; an unknown job name leads to the job selection list.",
            "days= sets the preview period, from -365 to 365, default 7; negative values look back. days=<from>,<to> gives a range relative to now. In text mode only the larger absolute value of the two counts as days from now; under .api 2 the range is applied exactly.",
        ],
        examples: &[
            ("status scheduler", "preview of the next seven days"),
            ("status scheduler schedule=WeeklyCycle days=14", "runs of one schedule over two weeks"),
            ("status scheduler client=bareos-fd", "only the jobs of one client"),
        ],
    },
    Doc {
        path: "status subscriptions",
        long: &[
            "Evaluates the backup units relevant for subscriptions from the catalog. clients adds a summary per client, plugins a summary per plugin, all every partial report; anonymize replaces client names by Client 1, Client 2 and so on. client= prints a detail report per fileset for one client.",
            "The command needs access to all client, job and fileset resources and is refused for consoles with restricted ACLs. The words detail and unknown are obsolete and are ignored with a notice.",
        ],
        examples: &[
            ("status subscriptions", "total backup units of all clients"),
            ("status subscriptions all anonymize", "every partial report with client names hidden"),
            ("status subscriptions client=bareos-fd", "units per fileset of one client"),
        ],
    },
    Doc {
        path: "status configuration",
        long: &[
            "Checks the loaded configuration for deprecated settings and lists them. The console needs the right to run the configure command; otherwise the Director refuses the report.",
        ],
        examples: &[
            ("status configuration", "list deprecated directives in the configuration"),
        ],
    },
    Doc {
        path: "run",
        long: &[],
        examples: &[
            ("run job=BackupClient1", "show the job settings and ask yes, mod or no"),
            ("run job=BackupClient1 level=Incremental pool=File when=\"2026-11-21 22:00:00\" yes", "schedule an incremental for tomorrow night"),
            ("run job=RestoreFiles jobid=42 where=/tmp/bareos-restores replace=never yes", "restore the files of job 42 without overwriting"),
        ],
    },
    Doc {
        path: "cancel",
        long: &[],
        examples: &[
            ("cancel jobid=42", "cancel job 42 after confirmation"),
            ("cancel job=BackupClient1 yes", "cancel the running BackupClient1 job without asking"),
            ("cancel state=waiting yes", "cancel every waiting job"),
        ],
    },
    Doc {
        path: "restore",
        long: &[],
        examples: &[
            ("restore", "open the selection menu"),
            ("restore jobid=42 client=bareos-fd restorejob=RestoreFiles where=/tmp/bareos-restores all done yes", "restore all files of job 42 to a separate directory"),
            ("restore client=bareos-fd restorejob=RestoreFiles file=/etc/fstab where=/tmp/bareos-restores yes", "restore a single file from the latest backup"),
        ],
    },
    Doc {
        path: "enable",
        long: &[
            "Switches a job, client or schedule back on that was switched off with disable. An enabled job is started by its schedule again, an enabled client is backed up again. Without an argument, or with job, client or schedule without a value, the Director offers the disabled resources of that kind; if there is exactly one it is selected automatically.",
            "The change lives only in the memory of the running Director and is lost at the next reload or restart. A permanent setting requires the Enabled directive in the configuration. The catalog is not changed.",
        ],
        examples: &[
            ("enable job=BackupClient1", "let the scheduler start BackupClient1 again"),
            ("enable client=bareos-fd", "back up the client bareos-fd again"),
            ("enable schedule=WeeklyCycle", "reactivate a schedule"),
        ],
    },
    Doc {
        path: "disable",
        long: &[
            "Switches a job, client or schedule off. A disabled job is no longer started by its schedule, a disabled client is no longer backed up, a disabled schedule triggers no jobs. A disabled job can still be started by hand with run. Without an argument, or with job, client or schedule without a value, the Director offers the enabled resources of that kind.",
            "The change lives only in the memory of the running Director until the next reload or restart. A permanent setting requires Enabled = no in the configuration.",
        ],
        examples: &[
            ("disable job=BackupClient1", "stop scheduled runs of BackupClient1 until the next reload"),
            ("disable client=bareos-fd", "skip the client during maintenance"),
            ("disable schedule=WeeklyCycle", "suspend a schedule"),
        ],
    },
    Doc {
        path: "estimate",
        long: &[
            "Asks the File Daemon how many files and bytes a backup job would save at the chosen level, without starting a job. The Director connects to the client, sends fileset and level and prints the answer; with listing every file that would be saved is printed as well. Without job= the Director offers the list of jobs.",
            "level=, client= (or fd=), fileset= and accurate= override the values of the job. Only native clients are supported, not NDMP. Nothing is changed; for Incremental and Differential the catalog is only read to find the last matching job.",
        ],
        examples: &[
            ("estimate job=BackupClient1 level=Full", "size of a full backup of BackupClient1"),
            ("estimate job=BackupClient1 level=Incremental listing", "every file the next incremental would save"),
        ],
    },
    Doc {
        path: "rerun",
        long: &[
            "Starts earlier jobs again with the parameters they ran with (job, level, client, fileset, pool). Jobs are chosen by JobId, by a JobId range with since_jobid= and until_jobid=, or by a time window with days= and hours=. For every job the Director shows the run summary and asks yes, mod or no; yes on the command line starts all chosen jobs without asking.",
            "since_jobid=, days= and hours= consider only jobs that ended with a fatal error (status f); for days= and hours= the scheduled start time counts, and both values add up. until_jobid= is only valid together with since_jobid=. Before rerunning several jobs found this way the Director lists the JobIds and asks for confirmation.",
        ],
        examples: &[
            ("rerun jobid=42", "run job 42 again with its original parameters"),
            ("rerun days=1 yes", "restart every job that failed during the last day"),
            ("rerun since_jobid=100 until_jobid=120", "failed jobs between JobId 100 and 120"),
        ],
    },
    Doc {
        path: "wait",
        long: &[
            "Blocks the session until jobs have finished. Without an argument it waits until no job is running. With jobid=, jobname= or job= (a prefix of a running job's name suffices) or ujobid= it waits for that one job and then prints JobId and JobStatus; a JobId that is not running is looked up in the catalog at once. Only the first such argument counts.",
            "With mount the command waits until a running job asks for a volume or a mount; timeout= limits that wait in seconds. Nothing is changed. The command is meant for scripts, for example after run ... yes and before evaluating the result.",
        ],
        examples: &[
            ("wait", "return when no job is running any more"),
            ("wait jobid=42", "wait for job 42 and print its final status"),
            ("wait mount timeout=300", "wait up to five minutes for a mount request"),
        ],
    },
    Doc {
        path: "messages",
        long: &[],
        examples: &[
            ("messages", "fetch job reports and mount requests queued for this console"),
        ],
    },
    Doc {
        path: "autodisplay",
        long: &[
            "The Director collects messages for the console, such as job reports or requests to mount a volume. With autodisplay off (the default) it only prints \"You have messages.\" after a command, and the messages are fetched with messages. With autodisplay on the Director prints pending messages itself after every command.",
            "The setting applies only to the own session and requires that the console may use the messages command. Case does not matter; the first word that reads on or off counts.",
        ],
        examples: &[
            ("autodisplay on", "print queued messages after every command"),
            ("autodisplay off", "only show the you have messages hint"),
        ],
    },
    Doc {
        path: "automount",
        long: &[
            "Decides for the current console session whether a volume is mounted automatically after label. The setting starts switched on, applies only to this session and changes neither catalog, configuration nor daemons. There is no confirmation.",
            "Only off (in any case) switches it off; every other word switches it on without an error. If not exactly one word follows, the Director asks \"Turn on or off?\" and evaluates the answer the same way.",
        ],
        examples: &[
            ("automount off", "label volumes without mounting them"),
            ("automount on", "mount every newly labelled volume"),
        ],
    },
    Doc {
        path: "gui",
        long: &[
            "gui on switches the session into a mode for graphical front ends and scripts, gui off back to the interactive default. In GUI mode the Director silently uses the first catalog of the configuration when no catalog= is given, prints catalog numbers without thousands separators and left aligned, and omits the \"You have messages.\" hint.",
            "The setting applies only to the own session until it ends. Case does not matter; the first word that reads on or off counts.",
        ],
        examples: &[
            ("gui on", "machine friendly output without catalog prompts"),
            ("gui off", "back to interactive mode"),
        ],
    },
    Doc {
        path: "memory",
        long: &[
            "Despite its description this command reports no memory usage. Like Bareos it prints only the header lines of status director: name, version, build date and operating system of the Director, start time, number of finished and running jobs, database type and binary kind. The command only reads; further words are ignored.",
        ],
        examples: &[
            ("memory", "header lines of the Director status"),
        ],
    },
    Doc {
        path: "trace",
        long: &[
            "trace on makes the Director write its debug output to the file bareos-dir.trace in its working directory; trace off stops this and closes the file. How much is written depends on the debug level set with setdebug. The setting affects the whole Director, not only the own session, and lasts until it is switched off or the Director restarts.",
            "Only off switches it off; every other word switches it on, including typos. Without exactly one word the Director asks \"Turn on or off?\"; a single dot aborts. Onesimus extension: if the trace file cannot be opened, odir reports the path and the error, whereas Bareos stays silent.",
        ],
        examples: &[
            ("trace on", "write debug output to bareos-dir.trace"),
            ("trace off", "stop tracing and close the file"),
        ],
    },
    Doc {
        path: "var",
        long: &[
            "Expands variables in a text the way the Director does for volume names (LabelFormat) and prints the result, so a LabelFormat can be tried before it goes into the configuration. Supported are date and time variables such as ${Year} and ${Month}, job variables such as ${Job}, ${Client}, ${Pool} and ${NumVols}, counters, environment variables, and operations like :u, :l, :p and :s/pattern/replacement/.",
            "The values come from the console job of the session: job, client, pool and storage are the defaults of the first job in the configuration, ${Type} is Console and ${JobId} is 0. The command opens the session catalog. ${Counter+} increments a counter permanently. Unknown variables stay unchanged. Without text odir prints an empty line.",
        ],
        examples: &[
            ("var Full-${Year}${Month}${Day}", "try a date based volume label"),
            ("var ${Pool}-${NumVols:p/4/0/r}", "pool name with the volume count padded to four digits"),
        ],
    },
    Doc {
        path: "whoami",
        long: &[],
        examples: &[
            ("whoami", "name of the console this session is logged in with"),
        ],
    },
    Doc {
        path: "use",
        long: &[
            "Sets the catalog the session works with. All following catalog commands such as list, query or sqlquery use it until the session ends or another one is chosen. An open catalog connection is closed and the new one opened; configuration and other sessions are not affected.",
            "Without catalog= the Director selects: with only one permitted catalog it takes that one, with several it asks (in GUI mode it takes the first). An unknown name, a name without the catalog= keyword and catalog without a value are treated like a missing catalog=.",
        ],
        examples: &[
            ("use catalog=MyCatalog", "work with the catalog MyCatalog for the rest of the session"),
        ],
    },
    Doc {
        path: "resolve",
        long: &[
            "Shows the IP addresses a host name resolves to. Without client= or storage= the Director resolves the name itself; with client= (or fd=) the File Daemon resolves it, with storage= the Storage Daemon, and the Director prints their answer. This helps to locate name resolution problems on either side of a connection. Several host names may be given; nothing is changed.",
            "When the Director resolves itself and a name fails, the command stops there and later names are not tried. Without any argument there is no output.",
        ],
        examples: &[
            ("resolve backup.example.com", "addresses as seen by the Director"),
            ("resolve client=bareos-fd backup.example.com", "how the File Daemon resolves the Director host"),
        ],
    },
    Doc {
        path: "setdebug",
        long: &[
            "Sets the debug level of the Director (dir or director), a File Daemon (client=), a Storage Daemon (storage=) or all daemons at once (all). Level 0 switches debug output off. The running daemon is changed, not its configuration; after a restart the start value applies again.",
            "trace= writes the debug output to a trace file in the working directory, timestamp= prefixes every debug line with a time stamp; without them the setting stays unchanged. hangup= is a File Daemon test aid that drops the connection after n files. A missing level is asked for, a missing target leads to a daemon selection.",
        ],
        examples: &[
            ("setdebug level=100 trace=1 dir", "verbose Director debugging into the trace file"),
            ("setdebug level=200 timestamp=1 client=bareos-fd", "debug the File Daemon with time stamps"),
            ("setdebug level=0 all", "switch debug output off everywhere"),
        ],
    },
    Doc {
        path: "setbandwidth",
        long: &[
            "Limits the bandwidth of a File Daemon, a Storage Daemon or running jobs without changing the configuration. The Director connects to the daemon and sends the new limit in kB/s; it applies until the daemon restarts or the job ends. limit=0 removes the limit; a missing limit is asked for. NDMP storages cannot be limited.",
            "job=, jobid=, ujobid=, all and state= select running jobs. For backup and restore the limit is applied through the File Daemon, for copy and migrate through the reading Storage Daemon. Without client, storage or a job selection the Director asks for the client.",
        ],
        examples: &[
            ("setbandwidth limit=10000 client=bareos-fd", "throttle the File Daemon to about 10 MB/s"),
            ("setbandwidth limit=5000 jobid=42", "slow down the running job 42"),
            ("setbandwidth limit=0 client=bareos-fd", "remove the limit again"),
        ],
    },
    Doc {
        path: "setip",
        long: &[
            "Meant for clients with changing IP addresses such as laptops. The client logs in to the Director through its own console whose name equals the name of its Client resource and calls setip; the Director then takes the source address of that console connection as the client's new address.",
            "The change applies only to the running Director; the configuration files stay unchanged. The command takes no arguments; further words are ignored.",
        ],
        examples: &[
            ("setip", "register the address of this console connection as the client address"),
        ],
    },
    Doc {
        path: "setdevice",
        long: &[
            "Changes the Auto Select setting of a device in the Storage Daemon at run time. With autoselect=no the Storage Daemon no longer picks the device for jobs, for example during maintenance; with autoselect=yes it is available again. The state lasts until the Storage Daemon restarts; configuration and catalog are not touched.",
            "storage=, device= and autoselect= are all required, in any order, and the keywords must be lower case; no other arguments are allowed. A console restricted to certain storages may only change their devices.",
        ],
        examples: &[
            ("setdevice storage=Tape device=Drive-1 autoselect=no", "keep jobs away from Drive-1 during maintenance"),
            ("setdevice storage=Tape device=Drive-1 autoselect=yes", "make the drive available again"),
        ],
    },
    Doc {
        path: "reload",
        long: &[],
        examples: &[
            ("reload", "apply edited configuration files without restarting odir"),
        ],
    },
    Doc {
        path: "list",
        long: &[],
        examples: &[
            ("list volumes pool=Full", "the volumes of the pool Full"),
            ("list jobs jobstatus=E,f days=7", "jobs that failed during the last week"),
        ],
    },
    Doc {
        path: "show",
        long: &[],
        examples: &[
            ("show jobs", "every job resource in configuration syntax"),
            ("show client=bareos-fd verbose", "one client including all directives at their default values"),
        ],
    },
    Doc {
        path: "sqlquery",
        long: &[],
        examples: &[
            ("sqlquery", "enter SQL statements ending with a semicolon, an empty line leaves the mode"),
        ],
    },
    Doc {
        path: "query",
        long: &[],
        examples: &[
            ("query", "pick a predefined catalog query by number and answer its questions"),
        ],
    },
    Doc {
        path: ".sql",
        long: &[
            "Sends an arbitrary SQL statement to the catalog database and prints the result. The WebUI and scripts use it for evaluations no other command provides. The statement is executed without any check: UPDATE, DELETE or DROP change the catalog permanently, so use plain SELECT statements for reading.",
            "If the text holds several statements separated by semicolons, all of them are executed but only the result of the last one is shown. Database errors are passed through as \"Query failed: ...\". In API mode 0 the result is a framed table, in mode 1 the cells are tab-separated on a single line, in mode 2 the rows are objects under \"query\" with all values as strings.",
        ],
        examples: &[
            (".sql query=\"SELECT poolid, name FROM pool\"", "ids and names of all pools"),
            (".sql query=\"SELECT count(*) FROM job WHERE jobstatus = 'E'\" catalog=MyCatalog", "number of failed jobs in the catalog MyCatalog"),
        ],
    },
    Doc {
        path: ".sqltables",
        long: &[],
        examples: &[
            (".sqltables", "the tables of the public schema in alphabetical order"),
            (".sqltables catalog=MyCatalog", "the tables of the catalog MyCatalog"),
        ],
    },
    Doc {
        path: ".sqlschema",
        long: &[
            "Shows every column of the tables in the public schema of the catalog database with its data type, ordered by table and column position; table= restricts the output to one table. An unknown table yields an empty list, a name that is not a plain identifier is rejected. The command only reads and helps writing queries for .sql.",
            "Onesimus extension: Bareos does not know the command and answers \".sqlschema: is an invalid command.\". Besides .sqlschema in its Command ACL the console needs .sql or sqlquery, as for any raw catalog access.",
        ],
        examples: &[
            (".sqlschema table=pool", "columns and data types of the pool table"),
            (".sqlschema", "the columns of every catalog table"),
        ],
    },
    Doc {
        path: ".sqlcount",
        long: &[
            "Counts the rows of catalog tables. Without an argument it reports the most important tables (client, fileset, job, jobmedia, media, pool, storage); File and Path are left out on purpose because counting them takes long in large catalogs. With table= exactly one table is counted. The command only reads.",
            "Onesimus extension: Bareos does not know the command and answers \".sqlcount: is an invalid command.\", so programs that must work with both directors should not rely on it. The console needs .sql or sqlquery in its Command ACL in addition to .sqlcount.",
        ],
        examples: &[
            (".sqlcount", "row counts of the main catalog tables"),
            (".sqlcount table=file", "number of rows in the file table"),
        ],
    },
    Doc {
        path: ".help",
        long: &[
            "Prints the command table of the Director in a form programs can parse easily. bconsole and oconsole build their completion from it and the WebUI uses it to check which commands a console may use; help is the variant for humans. Only commands allowed by the Command ACL of the console are listed, and only the commands of Bareos: Onesimus extensions such as .sqlcount or the .ovfs commands are missing (see .grammar).",
            "Without an argument or with full, every command is printed as one line \"name description -- arguments\". all prints only the names of the ordinary commands without dot commands, item= only the argument list of one command; an unknown name gives an empty reply. In API mode 2 the result is an object keyed by command name with command, description, arguments and permission.",
        ],
        examples: &[
            (".help item=list", "the argument list of list"),
            (".help all", "the names of all ordinary commands, one per line"),
        ],
    },
    Doc {
        path: ".quit",
        long: &[
            "Ends the console session: the Director closes the connection without sending text. It is the dot-command form of quit and exit meant for programs. Running jobs continue. Further words on the line are ignored.",
            "In API mode 1 the Director sends the signals CMD_BEGIN and CMD_OK before closing, in mode 2 additionally an empty JSON-RPC result; no MAIN_PROMPT follows in any mode. If the Command ACL does not allow .quit, the connection stays open.",
        ],
        examples: &[
            (".quit", "close the connection"),
        ],
    },
    Doc {
        path: ".",
        long: &[
            "A single dot as command does nothing and replies without text. Programs send it to check that the connection is alive or to make sure they are back at the main prompt after an error. Words after the dot are ignored; two dots are an unknown command.",
            "Inside a dialog the dot has a different role: as answer to a question of the Director, for example a selection list, it aborts the dialog. In API modes 1 and 2 the reply is framed twice (CMD_BEGIN, CMD_BEGIN, CMD_OK, CMD_OK, MAIN_PROMPT), which programs splitting replies at the signals must take into account.",
        ],
        examples: &[
            (".", "check that the connection is alive"),
            (". keepalive", "the same, trailing words are ignored"),
        ],
    },
    Doc {
        path: ".api",
        long: &[
            "Selects the reply format of the Director for this console session: mode 0 or off is text for humans (the default after login), mode 1 or on is terse text for scripts without table frames and catalog notices, mode 2 or json answers every command with a JSON-RPC 2.0 object. Without an argument it acts like .api 1. The mode lasts until the next .api or the end of the session.",
            "compact=yes after 2 or json prints the JSON on one line, any other value switches back to indented JSON; after 0 or 1 it is ignored. An unknown mode or more than two words leave the mode unchanged. In modes 1 and 2 every reply is framed by CMD_BEGIN, CMD_OK or CMD_FAILED and MAIN_PROMPT.",
            "Over gRPC each ExecuteCommand call carries its own response_format and the Director does not remember .api; oconsole --grpc remembers the mode chosen with .api and sends it with every following command, so .api behaves as over BNET.",
        ],
        examples: &[
            (".api 2", "switch to JSON replies"),
            (".api json compact=yes", "JSON replies on a single line"),
            (".api 0", "back to text mode"),
        ],
    },
    Doc {
        path: ".grpc",
        long: &[],
        examples: &[
            (".grpc", "the state of the session and whether the gRPC service is enabled"),
            (".grpc on compact=yes", "switch to gRPC mode with compact JSON"),
            (".grpc off", "return to the API mode used before .grpc on"),
        ],
    },
    Doc {
        path: ".grammar",
        long: &[],
        examples: &[
            (".grammar command=list", "only the grammar of list and its subcommands"),
        ],
    },
    Doc {
        path: ".status",
        long: &[
            "Returns one section of the status display of the Director, a Storage Daemon or a client, for example only the header or only the running jobs. The WebUI and monitoring scripts use it so they need not parse the full output of status, and in API mode 2 .status dir returns structured JSON. For storage= and client= the Director connects to the daemon.",
            "A target must follow: dir, storage=<name> or client=<name>; without one the reply is \"1900 Bad .status command, missing arguments.\" Unknown targets and a missing or unknown section under dir give \"1900 Bad .status command, wrong argument.\"",
        ],
        examples: &[
            (".status dir running", "the jobs currently running on the Director"),
            (".status storage=File devices", "the devices of the storage File"),
        ],
    },
    Doc {
        path: "show disabled",
        long: &[
            "Lists which jobs, clients and schedules are disabled. Without a further word all three lists are printed; jobs, clients or schedules limit the output to one list, any other word acts like none. In API mode 1 only the names are printed, in mode 2 the result is an object \"disabled\" with the lists jobs, clients and schedules.",
            "Deviation from Bareos: odir shows the configured state, so a job disabled at runtime with the disable command does not appear here, whereas Bareos lists it.",
        ],
        examples: &[
            ("show disabled", "disabled jobs, clients and schedules"),
            ("show disabled schedules", "only the disabled schedules"),
        ],
    },
    Doc {
        path: ".status dir",
        long: &[
            "Shows one section of the Director status: header (version, start time and job counters), scheduled, running or terminated jobs. current and last only reply with the short answer \"1000 OK .status\". A section is required.",
            "In API mode 2 the sections are JSON: result.header with director, version, release_date, os, binary_info, daemon_started, jobs_run, jobs_running and config_warnings, and result.scheduled, result.running and result.terminated as arrays.",
        ],
        examples: &[
            (".status dir header", "version, start time and job counters of the Director"),
            (".status dir scheduled", "the jobs scheduled for the coming days"),
        ],
    },
    Doc {
        path: ".status storage",
        long: &[
            "Shows the status of a Storage Daemon, which the Director contacts for this. Without a section the full status output is printed; header, waitreservation, devices, volumes, spooling, running and terminated select one part. In API mode 2 there is no JSON for this target and the result is empty.",
        ],
        examples: &[
            (".status storage=File header", "the header of the storage daemon behind File"),
            (".status storage=File", "the full status of the storage daemon"),
        ],
    },
    Doc {
        path: ".status client",
        long: &[
            "Shows the status of a File Daemon, which the Director contacts for this; header, running or terminated select the section. In API mode 2 there is no JSON for this target and the result is empty. If the client cannot be reached, the same connection messages as with status appear.",
        ],
        examples: &[
            (".status client=bareos-fd header", "version and start time of the client bareos-fd"),
            (".status client=bareos-fd terminated", "the jobs the client finished last"),
        ],
    },
    Doc {
        path: "list jobs",
        long: &[
            "Lists the jobs of the catalog, one row per job. The filters can be combined: job= or jobname=, client=, jobstatus= (letters such as T, E, f or names such as error, comma-separated), jobtype= (letter or exact name such as Backup), joblevel= or level= (F, I, D or Full, Incremental, Differential), volume=, pool=, days= and hours=. last keeps only the latest run per job name, count prints just the number of rows, reverse shows the newest first.",
            "limit= and offset= page through the result. For a console with ACLs the limit is applied before the rows the console may not see are removed, so a limited listing can come out shorter or even empty. An unknown jobtype or level is an error.",
        ],
        examples: &[
            ("list jobs client=bareos-fd last", "the latest run of every job name on bareos-fd"),
            ("list jobs level=Full jobstatus=T days=30", "successful full backups of the last 30 days"),
            ("list jobs count", "the number of jobs in the catalog"),
        ],
    },
    Doc {
        path: "list job",
        long: &[
            "Lists the jobs with the given job name; it works like list jobs job=<name> and accepts the same filters.",
        ],
        examples: &[
            ("list job=BackupClient1 limit=5", "the first five runs of BackupClient1"),
            ("list job=BackupClient1 jobstatus=E", "the failed runs of BackupClient1"),
        ],
    },
    Doc {
        path: "list jobid",
        long: &[
            "Lists exactly the job with this JobId; llist jobid= shows all its fields one below the other. An unknown JobId gives an empty reply, and list jobid without a value reports \"Missing jobid parameter\".",
        ],
        examples: &[
            ("list jobid=42", "the catalog row of job 42"),
        ],
    },
    Doc {
        path: "list joblog",
        long: &[
            "Prints the stored log of one job with time stamps; jobid= or ujobid= selects the job. count prints only the number of log lines. An unknown JobId or a job the console may not access gives \"jobid not found in db, access to job or client denied by ACL, or client not found in db\".",
        ],
        examples: &[
            ("list joblog jobid=42", "the complete log of job 42"),
            ("list joblog jobid=42 count", "the number of log lines of job 42"),
        ],
    },
    Doc {
        path: "list jobmedia",
        long: &[
            "Shows which volumes a job wrote, with the first and last file index on each volume; without jobid= or ujobid= the records of all jobs are listed. llist adds the media id, start and end file and block.",
        ],
        examples: &[
            ("list jobmedia jobid=42", "the volumes written by job 42"),
        ],
    },
    Doc {
        path: "list jobtotals",
        long: &[
            "Shows the number of jobs, files and bytes per job name for backup, archive and copy jobs, followed by the totals over all of them. In API mode 2 the rows are under \"jobs\" and the sums under \"jobtotals\".",
        ],
        examples: &[
            ("list jobtotals", "jobs, files and bytes per job name and in total"),
        ],
    },
    Doc {
        path: "list jobstatistics",
        long: &[
            "Lists the statistics samples (sample time, files and bytes) recorded for a running or finished job; jobid= is required. Without it, or for a job the console may not access, the reply is \"no jobid given\"; a job without statistics gives \"No results to list.\" The alias jobstats is accepted.",
        ],
        examples: &[
            ("list jobstatistics jobid=42", "the statistics samples of job 42"),
        ],
    },
    Doc {
        path: "list files",
        long: &[
            "Lists the paths of all files saved by one job, including the base files it refers to; jobid= or ujobid= is required. limit= and offset= do not shorten the text listing, they only appear in the range information of API mode 2.",
        ],
        examples: &[
            ("list files jobid=42", "every file saved by job 42"),
        ],
    },
    Doc {
        path: "list basefiles",
        long: &[
            "Lists the base files of a job that used a Base job, that is the files taken over from the base backup instead of being saved again; jobid= or ujobid= is required.",
        ],
        examples: &[
            ("list basefiles jobid=42", "the base files referenced by job 42"),
        ],
    },
    Doc {
        path: "list fileset",
        long: &[
            "Shows one FileSet record of the catalog, including the stored FileSet text and its MD5 sum. The record is selected by filesetid= or by the job that used it with jobid= or ujobid=; without one of them the command reports an error.",
        ],
        examples: &[
            ("list fileset jobid=42", "the FileSet used by job 42"),
            ("list fileset filesetid=3", "the FileSet record with id 3"),
        ],
    },
    Doc {
        path: "list filesets",
        long: &[
            "Lists every FileSet record of the catalog with id, name, MD5 sum, creation time and FileSet text. A FileSet gets a new record whenever its definition changes; current keeps only FileSets still defined in the configuration.",
        ],
        examples: &[
            ("list filesets", "all FileSet records of the catalog"),
            ("list filesets current", "only FileSets still present in the configuration"),
        ],
    },
    Doc {
        path: "list volumes",
        long: &[
            "Lists the volumes of the catalog grouped by pool. all prints them in one table, pool= restricts the listing to one pool, jobid= or ujobid= shows only the names of the volumes a job used, and count prints the number of volumes per pool. media is an alias with the same keywords.",
            "If the pool given with pool= is not in the catalog, the Director lists the defined pools and asks for a selection; a dot aborts. In API mode 2 there is no question and the error is returned directly.",
        ],
        examples: &[
            ("list volumes pool=Full", "the volumes of the pool Full"),
            ("list volumes all", "all volumes in a single table"),
            ("list volumes jobid=42", "the names of the volumes job 42 wrote to"),
        ],
    },
    Doc {
        path: "list volume",
        long: &[
            "Shows exactly one volume by its name, with status, bytes, retention, slot, media type and last write time. volumeid= and mediaid= select it by its media id instead.",
        ],
        examples: &[
            ("list volume=Full-0001", "the catalog record of Full-0001"),
        ],
    },
    Doc {
        path: "list media",
        long: &[
            "Alias of list volumes: without a value it lists the volumes grouped by pool and takes the same keywords all, pool=, jobid= and ujobid=; with a value, media=<name>, it shows that single volume like list volume=.",
        ],
        examples: &[
            ("list media pool=Incremental", "the volumes of the pool Incremental"),
            ("list media=Full-0001", "one volume by name"),
        ],
    },
    Doc {
        path: "list pools",
        long: &[
            "Lists every pool record of the catalog with id, name, number of volumes, maximum volumes, pool type and label format. llist adds all other fields of the records.",
        ],
        examples: &[
            ("list pools", "all pools of the catalog"),
        ],
    },
    Doc {
        path: "list pool",
        long: &[
            "Shows the catalog record of one pool by name; poolid= selects it by id instead.",
        ],
        examples: &[
            ("list pool=Full", "the catalog record of the pool Full"),
        ],
    },
    Doc {
        path: "list clients",
        long: &[
            "Lists every client record of the catalog with id, name, file retention and job retention; llist adds the uname and autoprune setting. enabled or disabled keep only clients in that state in the configuration, current only clients still configured.",
        ],
        examples: &[
            ("list clients", "all clients of the catalog"),
            ("list clients disabled", "clients disabled in the configuration"),
        ],
    },
    Doc {
        path: "list client",
        long: &[
            "Shows the catalog record of one client by name. list client without a value is not a valid keyword and gives \"Unknown list keyword\".",
        ],
        examples: &[
            ("list client=bareos-fd", "the catalog record of bareos-fd"),
        ],
    },
    Doc {
        path: "list storages",
        long: &[
            "Lists every storage record of the catalog. current, enabled and disabled restrict the listing to storages still configured or in that state.",
        ],
        examples: &[
            ("list storages", "all storages of the catalog"),
        ],
    },
    Doc {
        path: "list backups",
        long: &[
            "Lists the backups of one client; client= is required. fileset= limits the listing to one FileSet, fileset=any includes all of them. jobstatus=, level=, days= and hours= filter as with list jobs, order=asc or order=desc sorts by start time; any other order value is an error.",
            "Without client= the reply is \"missing parameter: client\". A FileSet outside the FileSet ACL of the console gives \"Access to specified FileSet not allowed.\"",
        ],
        examples: &[
            ("list backups client=bareos-fd fileset=any order=desc limit=5", "the five newest backups of bareos-fd"),
            ("list backups client=bareos-fd level=Full", "the full backups of bareos-fd"),
        ],
    },
    Doc {
        path: "list copies",
        long: &[
            "Lists copy jobs together with the job they were copied from and the media type. jobid= takes one JobId or a comma-separated list and limits the listing to copies of or made by these jobs. If there are no copy jobs, the reply is empty.",
        ],
        examples: &[
            ("list copies", "all copy jobs of the catalog"),
            ("list copies jobid=42,43", "the copies of jobs 42 and 43"),
        ],
    },
    Doc {
        path: "list nextvolume",
        long: &[
            "Shows which volume a job would write next, once for every scheduled run within the coming days; days= sets how many days to look ahead (at most 50, other values are ignored with a warning). Without job= the Director lists the job resources and asks for one; a dot aborts. For jobs that write no volume, such as restore jobs, it reports that no next volume can be found.",
            "Deviation from Bareos: for an unknown job odir reports success, whereas Bareos returns an error. In API mode 2 the result is empty.",
        ],
        examples: &[
            ("list nextvolume job=BackupClient1", "the next volume BackupClient1 will use"),
            ("list nextvolume job=BackupClient1 days=7", "the next volumes for the runs of the coming week"),
        ],
    },
    Doc {
        path: "list log",
        long: &[
            "Prints the most recent log lines of all jobs from the catalog, by default the last five; limit= changes the number. client= restricts them to jobs of one client, an unknown client gives \"invalid client argument\". The lines appear oldest first, reverse puts the newest first.",
        ],
        examples: &[
            ("list log limit=20", "the last 20 log lines of all jobs"),
            ("list log client=bareos-fd reverse", "the latest log lines of bareos-fd, newest first"),
        ],
    },
    Doc {
        path: "list schedules",
        long: &[
            "Schedules are configuration resources and have no table in the catalog, so like Bareos odir answers \"Unknown list keyword: schedules\". Use show schedules for the schedule resources and show disabled schedules for the disabled ones.",
        ],
        examples: &[
            ("list schedules disabled", "answered with Unknown list keyword, use show disabled schedules instead"),
        ],
    },
    Doc {
        path: "label",
        long: &[
            "Writes a Bareos label onto a new medium (file or tape) through the Storage Daemon and creates the matching Media record in the catalog. Only a labelled volume can be used for backups. The command changes both the medium and the catalog. Missing values are asked for: storage, volume name, slot and pool.",
            "Volume names may contain letters, digits and :.-_/ but must not start with /. slot= greater than 0 also sets the InChanger flag; on an autochanger without slot= the Director asks for the slot. drive= selects the drive and defaults to 0. encrypt creates a new key for hardware (LTO) encryption and stores it in the catalog.",
            "barcodes labels every volume of an autochanger with the name read from its barcode; slots= restricts this to a slot list such as 1-5,8, and yes answers the confirmation question. Only autochanger storages qualify.",
        ],
        examples: &[
            ("label storage=File volume=Full-0001 pool=Full", "label one file volume into pool Full"),
            ("label barcodes storage=Autochanger pool=Scratch slots=1-5 yes", "label slots 1 to 5 with their barcodes, without asking"),
        ],
    },
    Doc {
        path: "relabel",
        long: &[
            "Writes a new label onto a volume that is already in the catalog and gives it a new name. This works only for volumes with status Purged or Recycle, that is when no jobs refer to the volume any more. The Storage Daemon overwrites the medium, the old Media record is removed and a new one created. All data on the volume is lost.",
            "oldvolume= names the existing volume and volume= the new name, which follows the same rules as for label; missing or unknown names are chosen in a dialogue. pool= restricts the selection of the old volume. yes is accepted, but relabel asks no yes/no question.",
        ],
        examples: &[
            ("relabel storage=File oldvolume=Full-0001 volume=Full-0101", "relabel purged volume Full-0001 as Full-0101"),
        ],
    },
    Doc {
        path: "mount",
        long: &[
            "Asks the Storage Daemon to use a device again: on tape drives to read the loaded tape and make it available to jobs, on autochangers to load a volume from a slot first (slot=). Needed after unmount, after inserting a tape, or when a job waits for a volume. It changes the device state in the Storage Daemon, not the catalog. File devices are always mounted.",
            "The storage is given as storage=, sd= or by its bare name; without it a selection list appears. jobid=, job= or ujobid= use the storage of a running job. A keyword without a value is read as a storage name.",
            "Onesimus deviation: a job waiting for a volume whose Job resource has no Max Wait Time is cancelled after 10 minutes (unlimited in Bareos). Until then mount or a newly labelled volume lets the job continue.",
        ],
        examples: &[
            ("mount storage=Autochanger slot=3 drive=0", "load slot 3 into drive 0 and mount it"),
            ("mount jobid=42", "mount the storage the running job 42 waits on"),
        ],
    },
    Doc {
        path: "unmount",
        long: &[
            "Asks the Storage Daemon to stop using a device and release its volume, so a tape can be removed or the device serviced. While a device is unmounted no job writes to it; mount brings it back. It changes the device state in the Storage Daemon, not the catalog. umount is another name for the same command.",
            "The storage is given as storage=, sd= or by its bare name; without it a selection list appears. drive= picks one drive, alldrives unmounts every drive of the autochanger one after another. jobid=, job= or ujobid= use the storage of a running job.",
        ],
        examples: &[
            ("unmount storage=Autochanger drive=1", "unmount drive 1 of the autochanger"),
            ("unmount storage=Autochanger alldrives", "unmount every drive of the autochanger"),
        ],
    },
    Doc {
        path: "release",
        long: &[
            "Asks the Storage Daemon to release the volume in a device without unmounting the device. On tape drives the tape is rewound; the next job reads the label again and may request another volume. It changes the device state in the Storage Daemon, not the catalog.",
            "The storage is given as storage=, sd= or by its bare name; without it a selection list appears. drive= picks one drive, alldrives releases every drive of the storage. jobid=, job= or ujobid= use the storage of a running job.",
        ],
        examples: &[
            ("release storage=File", "release the volume of the File storage"),
            ("release storage=Autochanger alldrives", "release every drive of the autochanger"),
        ],
    },
    Doc {
        path: "update",
        long: &[],
        examples: &[
            ("update", "choose what to update from the menu"),
            ("update volume=Full-0001 volstatus=Used", "set the status of one volume"),
        ],
    },
    Doc {
        path: "delete",
        long: &[],
        examples: &[
            ("delete", "choose what to delete from the menu"),
            ("delete jobid=507-509,511", "delete four job records"),
        ],
    },
    Doc {
        path: "prune",
        long: &[],
        examples: &[
            ("prune", "choose what to prune from the menu"),
            ("prune jobs client=bareos-fd yes", "remove expired job records of bareos-fd"),
        ],
    },
    Doc {
        path: "purge",
        long: &[],
        examples: &[
            ("purge", "choose what to purge from the menu"),
            ("purge jobs client=bareos-fd jobstatus=E", "purge the failed jobs of bareos-fd"),
        ],
    },
    Doc {
        path: "truncate",
        long: &[
            "Frees the space of volumes whose jobs have already been removed (status Purged). The Director lists the matching volumes, asks for confirmation and has the Storage Daemon relabel them, which shrinks the volume file or tape to its label. The volumes stay in the catalog. Only native (non-NDMP) storages are supported, and only the volumes of one storage per call.",
            "volstatus=Purged is required. storage=, pool= and volume= narrow the selection; an unknown storage or pool opens a selection list. drive= selects the drive, yes skips the question. Other keywords, including client= and catalog=, end with Invalid parameter. Under API 1 and 2 nothing is asked, so without yes nothing happens.",
            "Onesimus deviation: a volume holding jobs of the latest Full chain of a client and FileSet is never truncated.",
        ],
        examples: &[
            ("truncate volstatus=Purged storage=File pool=Full yes", "truncate all purged volumes of pool Full on File"),
            ("truncate volstatus=Purged volume=Full-0003", "truncate one purged volume after confirmation"),
        ],
    },
    Doc {
        path: "create",
        long: &[
            "Creates the catalog Pool record for a Pool resource of the Director configuration. The Director normally does this at startup; create is needed when a pool was newly configured and must be in the catalog before the next job. Only the catalog is changed.",
            "Without a valid pool= the Director lists the configured pools in alphabetical order and asks for one; . aborts. To bring an existing Pool record in line with a changed configuration use update pool instead.",
        ],
        examples: &[
            ("create pool=Incremental", "create the catalog record of pool Incremental"),
        ],
    },
    Doc {
        path: "add",
        long: &[
            "Creates Media records for volumes in the catalog without labelling the media. This is only needed in special cases, for example when tapes already carry a Bareos label or volumes should be listed in advance. The Storage Daemon is not contacted. Normally label is the right command, and the Director says so on every call.",
            "pool= selects the pool (asked for if missing, automatic if the console may use only one pool); storage= determines Media Type and storage of the new volumes. Number, name, start number and, on autochangers, slot and InChanger are always asked in the dialogue: 0 creates one volume with a fixed name, a larger number a series with a four-digit suffix. jobid= is accepted but ignored.",
        ],
        examples: &[
            ("add pool=Full storage=File", "add volumes to pool Full for storage File"),
        ],
    },
    Doc {
        path: "import",
        long: &[
            "Has an autochanger move volumes from its import/export slots (mail slots) into free normal slots. The Director instructs the Storage Daemon, which moves the robot, and then records the slots in the catalog. It works only with storages configured as autochangers; the counterpart is export.",
            "srcslots= lists occupied import/export slots (default: all occupied ones), dstslots= empty normal slots (default: free slots in order); slot lists use numbers from 1, ranges with - and commas. volume= restricts the import to names separated by |. scan reads the tapes in a drive instead of using barcodes and asks for the drive. Onesimus deviation: on a storage without autochanger the command is sent to the Storage Daemon, which rejects it.",
        ],
        examples: &[
            ("import storage=Autochanger", "import every volume from the mail slots"),
            ("import storage=Autochanger srcslots=41-42 dstslots=10-11", "move mail slots 41 and 42 into slots 10 and 11"),
        ],
    },
    Doc {
        path: "export",
        long: &[
            "Has an autochanger move volumes from normal slots into its import/export slots (mail slots) so the tapes can be removed, for example for offsite storage. The Director instructs the Storage Daemon, which moves the robot, and then records the new slots in the catalog. It works only with storages configured as autochangers.",
            "A source is required: srcslots= with occupied normal slots or volume= with names separated by |; empty source slots are skipped silently. dstslots= gives empty import/export slots, by default free ones in order. scan reads the tapes in a drive instead of using barcodes. Onesimus deviation: without a source the Director refuses before contacting the Storage Daemon.",
        ],
        examples: &[
            ("export storage=Autochanger srcslots=1-3", "export the volumes in slots 1 to 3"),
            ("export storage=Autochanger volume=Full-0001|Full-0002", "export two volumes by name"),
        ],
    },
    Doc {
        path: "move",
        long: &[
            "Has an autochanger move volumes from one slot to another. Source and destination are slot lists; the Director first asks the Storage Daemon which slots are occupied and moves only occupied source slots into free destination slots. The command changes the autochanger contents and the slot data in the catalog. It works only with autochanger storages.",
            "Without storage= the Director offers the autochanger storages. Empty slots in srcslots= are skipped; without a source or destination selection move does nothing.",
        ],
        examples: &[
            ("move storage=Autochanger srcslots=1-3 dstslots=10-12", "move slots 1 to 3 into slots 10 to 12"),
        ],
    },
    Doc {
        path: ".clients",
        long: &[
            "Prints the names of all Client resources of the Director configuration, one per line, without a header. Meant for programs: bconsole and oconsole complete client= with it, the WebUI fills its selection lists. It reads only the configuration and changes nothing.",
            "Only clients allowed by the console's Client ACL appear, in configuration order. enabled or disabled filter by state; a value after the keyword is ignored, and both together give an empty list.",
        ],
        examples: &[
            (".clients", "list all visible clients"),
            (".clients enabled", "list only enabled clients"),
        ],
    },
    Doc {
        path: ".jobs",
        long: &[
            "Prints the names of all Job resources of the Director configuration, one per line, for completion of job= and selection lists. Only jobs allowed by the console's Job ACL appear, in configuration order. It reads only the configuration.",
            "type= compares only the first letter of its value, case-sensitively, with the job type letter (B Backup, R Restore, V Verify, D Admin, c Copy, g Migrate, O Consolidate, A Archive); so type=Backup works but type=backup returns nothing. enabled and disabled filter as for .clients.",
        ],
        examples: &[
            (".jobs type=Backup", "list the backup jobs"),
            (".jobs disabled", "list only disabled jobs"),
        ],
    },
    Doc {
        path: ".pools",
        long: &[
            "Prints the names of all Pool resources of the Director configuration, one per line, for completion of pool= and selection lists. It reads the configuration, not the catalog Pool table. Only pools allowed by the console's Pool ACL appear.",
            "type= keeps pools whose type starts with the given text, ignoring case: type=b and type=BACKUP give the same result. Without a value no filter applies.",
        ],
        examples: &[
            (".pools type=Scratch", "list the scratch pools"),
        ],
    },
    Doc {
        path: ".storage",
        long: &[
            "Not a command of its own but an abbreviation of .storages: like Bareos, the Director accepts any unique prefix of a command name and runs the first matching command. Output, arguments, API modes and permissions are those of .storages. It reads only the configuration; enabled or disabled filter by state.",
        ],
        examples: &[
            (".storage", "list the storage resources"),
            (".storage enabled", "list only enabled storages"),
        ],
    },
    Doc {
        path: ".filesets",
        long: &[
            "Prints the names of all FileSet resources of the Director configuration, one per line, for completion of fileset= (for example with run or restore) and selection lists. Only FileSets allowed by the console's FileSet ACL appear. It takes no arguments, reads only the configuration and changes nothing.",
        ],
        examples: &[
            (".filesets", "list all visible filesets"),
        ],
    },
    Doc {
        path: ".schedule",
        long: &[
            "Prints the names of all Schedule resources of the Director configuration, one per line, for selection lists such as enabling or disabling a schedule. Only schedules allowed by the console's Schedule ACL appear. enabled or disabled filter by state; a value after the keyword is ignored and both together give an empty list.",
        ],
        examples: &[
            (".schedule", "list all visible schedules"),
            (".schedule disabled", "list only disabled schedules"),
        ],
    },
    Doc {
        path: ".catalogs",
        long: &[
            "Prints the names of all Catalog resources of the Director configuration, one per line, for completion of catalog=. Only catalogs allowed by the console's Catalog ACL appear. It takes no arguments, opens no database connection and changes nothing.",
        ],
        examples: &[
            (".catalogs", "list all visible catalogs"),
        ],
    },
    Doc {
        path: ".consoles",
        long: &[
            "Prints the names of all Console resources, one per line, for user interfaces and tab completion. The list includes the built-in default console *UserAgent* and consoles created at runtime with configure. The order is the internal configuration order, not alphabetical, and the list is not filtered by ACLs. It takes no arguments and only reads.",
        ],
        examples: &[
            (".consoles", "list all consoles"),
        ],
    },
    Doc {
        path: ".jobdefs",
        long: &[
            "Prints the names of all JobDefs resources of the Director configuration, one per line. JobDefs are templates from which jobs inherit default values; configuration tools need the list when creating jobs. Only JobDefs allowed by the console's Job ACL appear. It takes no arguments and changes nothing.",
        ],
        examples: &[
            (".jobdefs", "list all job templates"),
        ],
    },
    Doc {
        path: ".profiles",
        long: &[
            "Prints the names of all Profile resources, one per line. Profiles bundle ACLs that Console and User resources take over with Profile = <name>. The order is the internal configuration order and the list is not filtered by ACLs. It takes no arguments and only reads.",
        ],
        examples: &[
            (".profiles", "list all profiles"),
        ],
    },
    Doc {
        path: ".locations",
        long: &[
            "Prints the locations from the catalog Location table, sorted alphabetically and without duplicates. Locations describe where a tape physically is, for example a safe or an offsite store. It only reads the catalog; without locations the list is empty.",
        ],
        examples: &[
            (".locations", "list the tape locations"),
        ],
    },
    Doc {
        path: ".media",
        long: &[
            "Prints the names of all volumes in the catalog Media table, sorted alphabetically and without duplicates. Programs use the list for completion of volume= and for selection lists. It only reads the catalog and changes nothing.",
        ],
        examples: &[
            (".media", "list all volume names"),
        ],
    },
    Doc {
        path: ".mediatypes",
        long: &[
            "Prints the media types recorded in the catalog MediaType table, sorted alphabetically and without duplicates. Programs use the list, for example when labelling or adding volumes. It only reads the catalog and changes nothing.",
        ],
        examples: &[
            (".mediatypes", "list the media types"),
        ],
    },
    Doc {
        path: ".messages",
        long: &[
            "Prints the messages queued for the console since they were last fetched (job reports, daemon warnings) and empties the queue. Unlike messages it prints nothing at all when nothing is waiting, which suits programs that poll regularly, such as bconsole and the WebUI. It works regardless of autodisplay and gui and takes no arguments.",
        ],
        examples: &[
            (".messages", "fetch pending messages, silent if there are none"),
        ],
    },
    Doc {
        path: ".msgs",
        long: &[
            "Prints the names of all Messages resources of the Director configuration, one per line. A Messages resource defines where job and daemon messages go (console, file, mail, catalog); configuration tools need the list for messages=. It takes no arguments and changes nothing. Not to be confused with .messages, which prints pending messages.",
        ],
        examples: &[
            (".msgs", "list the messages resources"),
        ],
    },
    Doc {
        path: ".users",
        long: &[
            "Prints the names of all User resources, one per line. User resources describe users who log in through a console with PAM authentication; their rights come from profiles. Without User resources the answer is empty. The order is the internal configuration order and the list is not filtered by ACLs. It takes no arguments and only reads.",
        ],
        examples: &[
            (".users", "list the PAM users"),
        ],
    },
    Doc {
        path: ".defaults",
        long: &[
            "Shows the main settings of a job, client, storage or pool as key=value lines. The WebUI and scripts use it to prefill forms such as run or restore with values from the configuration. It reads only the configuration and changes nothing.",
            "Give exactly one resource per call; the name must match exactly. A JobDefs template is not a job and returns nothing, nor does a resource hidden by the console's ACL. For pool= one line reads purge_oldest_volume==0 with a double equals sign, as in Bareos. Under API 2 an error leaves result empty instead of an empty defaults object.",
        ],
        examples: &[
            (".defaults job=backup-bareos-fd", "show the defaults of a job"),
            (".defaults pool=Full", "show the settings of pool Full"),
        ],
    },
    Doc {
        path: ".jobstatus",
        long: &[
            "Shows the table of job status codes from the catalog Status table: code letter (T, E, f, ...), description, severity, and exit level and exit status for finished states. Programs use it to explain the one-letter status of list jobs. It only reads the catalog.",
            "The filter must be attached to the command word (.jobstatus=T); a separate word is ignored and gives the whole table. catalog= selects a catalog; an unknown name is ignored and the first catalog is used.",
        ],
        examples: &[
            (".jobstatus", "show all job status codes"),
            (".jobstatus=T", "show only the row for code T"),
        ],
    },
    Doc {
        path: ".authorized",
        long: &[
            "Checks whether the own console may access a resource or command and answers authorized or not authorized. The check uses the ACLs of the Console resource and its profiles (Job, Client, Storage, Schedule, Pool, Command, FileSet and Catalog ACL). User interfaces such as bareos-webui use it to show buttons only when permitted. It only reads.",
            "Keys are case-insensitive, and only the ACL is checked, not whether the resource exists. With several arguments the last known one decides. Without arguments, with unknown keys only or with a key lacking a value the answer is not authorized, even for an unrestricted console.",
        ],
        examples: &[
            (".authorized cmd=purge", "check whether the console may run purge"),
            (".authorized client=bareos-fd", "check access to client bareos-fd"),
        ],
    },
    Doc {
        path: ".levels",
        long: &[
            "Prints the levels allowed for jobs, such as Full, Incremental or VirtualFull for backups and Catalog or VolumeToCatalog for verify jobs. Programs fill the level= selection of run or estimate with it. The table is built into the Director; nothing is read from configuration or catalog.",
            "Without an argument the whole table is printed, so Full, Incremental and Differential appear several times (once each for Backup, Copy and Migrate). A job type given as a single bare word, written out in full in any case, restricts the list; a prefix, more than one word or the form type=Backup gives an empty list, as do job types without levels.",
        ],
        examples: &[
            (".levels", "list all job levels"),
        ],
    },
    Doc {
        path: ".types",
        long: &[
            "Prints all job types the Director knows (Backup, Restore, Verify and so on). Programs fill selection lists with it, for example type= of .jobs or jobtype= of list jobs. The list is built into the Director; it takes no arguments and reads neither configuration nor catalog.",
        ],
        examples: &[
            (".types", "list all job types"),
        ],
    },
    Doc {
        path: ".volstatus",
        long: &[
            "Prints all states a volume can have in the catalog (Append, Full, Used, Purged and so on). Programs fill the volstatus= selection of update volume or truncate with it. The list is built into the Director; it takes no arguments and reads neither configuration nor catalog.",
        ],
        examples: &[
            (".volstatus", "list all volume states"),
        ],
    },
    Doc {
        path: ".actiononpurge",
        long: &[
            "Prints the values allowed for actiononpurge= (in update volume, update pool and the Pool resource): None, do nothing, and Truncate, shrink the volume file after the purge to free space. The list is built into the Director; it takes no arguments and reads neither configuration nor catalog.",
        ],
        examples: &[
            (".actiononpurge", "list the actions on purge"),
        ],
    },
    Doc {
        path: "update volume",
        long: &[
            "Changes the catalog properties of one volume: volstatus (Append, Archive, Disabled, Full, Used, Cleaning, Recycle, Read-Only, Error), volretention, voluse, maxvoljobs, maxvolfiles, maxvolbytes, recycle, inchanger, slot, enabled, encrypt, pool, recyclepool, actiononpurge and storage. Several parameters may be given on one line; each is checked and reported on its own. Without parameters a menu opens, without a name the volume is selected first.",
            "allfrompool=<pool> resets the defaults of all volumes of a pool from the Pool resource, fromallpools those of every pool. encrypt=yes|no|rotate sets, removes or renews the hardware encryption key and only applies to volumes with a key.",
        ],
        examples: &[
            ("update volume=Full-0001 volretention=30days maxvolbytes=10G recycle=no", "change several limits at once"),
            ("update volume=Full-0001 pool=Incremental", "move a volume to another pool"),
            ("update volume allfrompool=Full", "reapply the pool defaults to all volumes of Full"),
        ],
    },
    Doc {
        path: "update pool",
        long: &[
            "Updates the catalog Pool record from the Pool resource of the configuration and shows the record afterwards. Use it after changing a pool in the configuration; without a name the pool is chosen from a list. To create a missing Pool record use create.",
        ],
        examples: &[
            ("update pool=Full", "refresh the catalog record of pool Full"),
        ],
    },
    Doc {
        path: "update slots",
        long: &[
            "Asks the Storage Daemon for the contents of an autochanger and records slot and InChanger for the volumes found. It is the only form of update that contacts the Storage Daemon. Without storage= only autochanger storages are offered; a slot list restricts the update.",
            "scan loads volumes one by one into the drive given by drive= and reads their labels, for changers without a barcode reader. enabled= sets the Enabled value of the volumes found.",
        ],
        examples: &[
            ("update slots storage=Autochanger", "refresh all slots of the autochanger"),
            ("update slots=1-3,7 storage=Autochanger scan drive=0", "read the labels in slots 1 to 3 and 7"),
        ],
    },
    Doc {
        path: "update jobid",
        long: &[
            "Changes fields of one job record without confirmation: jobname, starttime (YYYY-MM-DD hh:mm:ss, quoted because of the blank), client, filesetid and jobtype. Without any field the Director reports that nothing was specified; an unknown JobId is reported by the catalog.",
        ],
        examples: &[
            ("update jobid=506 jobname=DocsRenamed", "rename the job of a record"),
            ("update jobid=506 starttime=\"2026-10-01 10:00:00\"", "correct the start time of a job"),
        ],
    },
    Doc {
        path: "update stats",
        long: &[
            "Carries the long-term job statistics forward by copying finished jobs into the statistics tables. days= takes only jobs of the last given number of days into account. The Director reports how many jobs were updated.",
        ],
        examples: &[
            ("update stats days=30", "update the statistics with the last 30 days"),
        ],
    },
    Doc {
        path: "delete volume",
        long: &[
            "Deletes a volume and every job stored on it from the catalog. The data on the medium is untouched, but it can no longer be selected for restore. Without a name the pool and volume are selected in a dialogue; pool= sets the pool for this selection. The Director asks for confirmation unless yes is given; the action cannot be undone.",
            "Onesimus deviation: a volume holding jobs of the latest Full chain of a client and FileSet is never deleted, not even with yes.",
        ],
        examples: &[
            ("delete volume=Full-0001 yes", "delete a volume without asking"),
            ("delete volume pool=Scratch", "choose the volume to delete from pool Scratch"),
        ],
    },
    Doc {
        path: "delete pool",
        long: &[
            "Deletes a Pool record from the catalog after confirmation; without a name the pool is chosen from a list. Onesimus deviation: deleting pools is not implemented yet, and after yes the Director answers delete pool: not supported.",
        ],
        examples: &[
            ("delete pool=Scratch", "ask to delete the Scratch pool record"),
        ],
    },
    Doc {
        path: "delete jobid",
        long: &[
            "Deletes jobs together with their file and JobMedia records from the catalog. The value may be a comma-separated list with ranges such as 507-509,511; a range of more than 25 JobIds requires confirmation. Without a value the Director asks for the JobId. This is also the way to remove a single job of a protected latest Full chain.",
        ],
        examples: &[
            ("delete jobid=14", "delete one job record"),
            ("delete jobid=600,602", "delete two job records"),
        ],
    },
    Doc {
        path: "delete storage",
        long: &[
            "Deletes an orphaned Storage record from the catalog, one that no longer exists in the configuration. A storage still configured is refused; remove it from the configuration first. Without a value the Director asks for the name.",
        ],
        examples: &[
            ("delete storage=OldTape", "remove the catalog record of a removed storage"),
        ],
    },
    Doc {
        path: "prune files",
        long: &[
            "Removes file records whose File Retention has expired, for one client (client=) or for the retention of a pool (pool=); without client= the client is chosen from a list. The Director shows the retention period and asks yes/mod/no, where mod sets a different period for the running Director until the next reload; yes skips the question. Full access to the involved resources is required.",
            "Onesimus deviation: the file records of the latest Full chain of every client and FileSet are always kept.",
        ],
        examples: &[
            ("prune files client=bareos-fd yes", "prune expired file records of bareos-fd"),
        ],
    },
    Doc {
        path: "prune jobs",
        long: &[
            "Removes job records whose Job Retention has expired, for one client (client=), a pool (pool=) or a job type (jobtype=). The Director shows the retention period and asks yes/mod/no unless yes is given. Full access to the involved resources is required.",
            "Onesimus deviation: the latest successful Full chain of every client and FileSet name is never pruned, even if it is older than the retention and even when job, FileSet or client are no longer configured. If that chain cannot be determined, nothing is deleted.",
        ],
        examples: &[
            ("prune jobs pool=Incremental yes", "prune expired jobs using the retention of pool Incremental"),
        ],
    },
    Doc {
        path: "prune volume",
        long: &[
            "Removes the jobs of a volume whose Volume Retention has expired; the volume must have status Full or Used. Without a name the pool and volume are selected in a dialogue. all prunes every volume, of one pool with pool=. The Director asks yes/mod/no unless yes is given.",
            "Onesimus deviation: a volume holding jobs of the latest Full chain of a client and FileSet is never pruned.",
        ],
        examples: &[
            ("prune volume=Full-0001 yes", "prune one volume without asking"),
            ("prune volume all pool=Full", "prune every volume of pool Full after confirmation"),
        ],
    },
    Doc {
        path: "prune stats",
        long: &[
            "Removes entries from JobHisto, DeviceStats and JobStats that are older than the Statistics Retention. The Director shows the retention period and asks unless yes is given.",
        ],
        examples: &[
            ("prune stats yes", "prune old statistics without asking"),
        ],
    },
    Doc {
        path: "prune directory",
        long: &[
            "Removes the file records below one directory from the catalog; recursive includes subdirectories. Without client= the Director first asks whether the directory should be removed for all clients, and without a path it asks for the full path. yes skips the confirmation.",
        ],
        examples: &[
            ("prune directory=/var/cache client=bareos-fd recursive yes", "drop /var/cache and below from the catalog of bareos-fd"),
        ],
    },
    Doc {
        path: "purge files",
        long: &[
            "Deletes file records from the catalog regardless of retention periods, for one job (job= or jobid=), a client or a volume; the jobs themselves stay. Like every purge it prints a warning, needs full access to all resources and cannot be undone.",
            "Onesimus deviation: the file records of the latest Full chain of a client and FileSet are skipped, and each purge runs in one transaction whose errors are reported instead of ignored.",
        ],
        examples: &[
            ("purge files jobid=599", "purge the file records of job 599"),
            ("purge files client=bareos-fd yes", "purge all file records of bareos-fd"),
        ],
    },
    Doc {
        path: "purge jobs",
        long: &[
            "Deletes whole jobs with all their records regardless of retention periods, for a client, a volume or a pool. jobstatus= limits this to jobs with the given status letters (T, E, f, ...) or names such as error, separated by commas. The Director counts the jobs and asks Purge (yes/no)? unless yes is given. A volume left without jobs becomes Purged.",
            "Onesimus deviation: jobs of the latest Full chain of a client and FileSet are skipped, and each purge runs in one transaction whose errors are reported.",
        ],
        examples: &[
            ("purge jobs volume=Full-0001 yes", "purge every job on volume Full-0001"),
            ("purge jobs pool=Incremental jobstatus=E,f", "purge the failed jobs of pool Incremental"),
        ],
    },
    Doc {
        path: "purge volume",
        long: &[
            "Deletes every job on a volume from the catalog regardless of retention; the volume then becomes Purged and can be reused. Several volume= may be given, and each is confirmed on its own unless yes is given. action=truncate or all runs the ActionOnPurge on Purged volumes through the Storage Daemon, with storage=, pool= or allpools, devicetype= and drive=. A volume holding the latest Full chain of a client is never purged (Onesimus).",
            "Onesimus extension: purge volume all purges every volume of pool= or of all permitted pools (allpools) whose status is in volstatus=, by default Full, Used and Error. Purged and Recycle volumes are never selected and Cleaning tapes cannot be purged. It never asks: exactly one of dryrun (preview) and yes is required, and refused volumes are skipped with their reason.",
        ],
        examples: &[
            ("purge volume action=truncate storage=File pool=Full", "truncate the purged volumes of pool Full"),
            ("purge volume all pool=Incremental volstatus=Error dryrun", "preview purging the Error volumes of Incremental (Onesimus)"),
        ],
    },
    Doc {
        path: "purge quota",
        long: &[
            "Resets the quota counters of a client; without client= the client is chosen from a list. Like every purge it prints a warning and needs full access to all resources.",
        ],
        examples: &[
            ("purge quota client=bareos-fd", "reset the quota of bareos-fd"),
        ],
    },
    Doc {
        path: "configure",
        long: &[],
        examples: &[
            ("configure export client=bareos-fd", "write the director resource for the file daemon of bareos-fd"),
            ("configure delete schedule=OldNightly", "remove an unused schedule (Onesimus)"),
        ],
    },
    Doc {
        path: ".acl",
        long: &[],
        examples: &[
            (".acl clear console=ops", "remove all catalog ACL entries of a console"),
            (".acl copy from=ops to=operator", "give a console the catalog ACL lists of another"),
        ],
    },
    Doc {
        path: ".config",
        long: &[],
        examples: &[
            (".config list type=job json", "the job resources as one JSON line"),
            (".config read type=schedule name=WeeklyCycle", "the raw text of a schedule with its checksum"),
        ],
    },
    Doc {
        path: ".bvfs_update",
        long: &[
            "Fills the BVFS cache in the catalog (the tables PathHierarchy and PathVisibility and the flag Job.HasCache). Without this cache .bvfs_lsdirs and .bvfs_lsfiles find no directories. With jobid= only the listed jobs are prepared; without it all jobs without cache are, which can take long on large catalogs. Jobs that already have a cache are skipped. Backup data is not changed.",
            "On success nothing is printed. BVFS is kept unchanged for compatibility with Bareos tools such as bareos-webui; the OVFS commands need no cache.",
        ],
        examples: &[
            (".bvfs_update jobid=600,615,616", "build the cache for one job chain"),
            (".bvfs_update", "build the cache for all jobs without one"),
        ],
    },
    Doc {
        path: ".bvfs_cleanup",
        long: &[
            "Drops the table that .bvfs_restore created for a restore selection, after the restore was started with file=?<table> or abandoned. path= names the table (b2 followed by a number); other names are skipped silently and a missing table is not an error. The BVFS cache stays.",
        ],
        examples: &[
            (".bvfs_cleanup path=b2101", "drop a restore selection table"),
        ],
    },
    Doc {
        path: ".bvfs_clear_cache",
        long: &[
            "Clears the whole BVFS cache in the catalog: Job.HasCache is reset for all jobs and the tables PathHierarchy and PathVisibility are emptied. The keyword yes is required; without it nothing happens and there is no confirmation prompt. Afterwards the cache must be rebuilt with .bvfs_update. Backup data and file records are not touched.",
        ],
        examples: &[
            (".bvfs_clear_cache yes", "empty the BVFS cache"),
        ],
    },
    Doc {
        path: ".bvfs_get_jobids",
        long: &[
            "Returns the chain of JobIds needed to restore the state of a job: the Full backup, the last Differential and all Incrementals up to that job, as one comma separated line. The list is the jobid= argument of the other .bvfs commands. ujobid= gives the job by its unique job name instead and takes precedence. all combines the chains of all filesets of the client.",
            "For a Base job only its own JobId is returned. JobIds the console may not see by its job, client or fileset ACL are left out. The command only reads the catalog; the OVFS counterpart is .ovfs_latest.",
        ],
        examples: &[
            (".bvfs_get_jobids jobid=628", "the job chain for restoring job 628"),
            (".bvfs_get_jobids jobid=628 all", "the chains of all filesets of the client"),
        ],
    },
    Doc {
        path: ".bvfs_lsdirs",
        long: &[
            "Lists the subdirectories of a directory in the state of a job chain, as a restore file browser needs them. The BVFS cache must have been filled with .bvfs_update first. The directory is given by path= (with a trailing slash, / for the root) or by pathid= from an earlier output; limit= (default 2000) and offset= page through the result.",
            "Each line has five tab separated columns: PathId, FileId, JobId, LStat and name. The first two lines are . and ..; directories without their own catalog record have FileId and JobId 0. A JobId the console may not see is refused.",
        ],
        examples: &[
            (".bvfs_lsdirs jobid=600,615,616 path=/", "the top level directories of a job chain"),
            (".bvfs_lsdirs jobid=600,615,616 pathid=7 limit=100", "the first 100 subdirectories of a known path"),
        ],
    },
    Doc {
        path: ".bvfs_lsfiles",
        long: &[
            "Lists the files (without subdirectories) of a directory in the state of a job chain; for each file the version from the newest job of the chain is shown. The FileIds are the input for .bvfs_restore. The BVFS cache must have been filled with .bvfs_update first. pattern= keeps only file names matching a regular expression; limit= (default 2000) and offset= page through the result.",
            "Each line has five tab separated columns: PathId, FileId, JobId, LStat and file name.",
        ],
        examples: &[
            (".bvfs_lsfiles jobid=600,615,616 path=/etc/", "the files of /etc in a job chain"),
            (".bvfs_lsfiles jobid=600,615,616 pathid=7 pattern=conf", "only file names matching a pattern"),
        ],
    },
    Doc {
        path: ".bvfs_versions",
        long: &[
            "Lists all backed up versions of one file of a client across all jobs, each with its volume, so an older version can be picked for .bvfs_restore by its FileId. Only versions stored on a volume count. jobid= is required but not evaluated (by Bareos convention 0); the directory is given by path= or pathid=, the file by fname= (also filename= or name=). copies includes versions from copy jobs; versions is accepted but has no effect.",
            "Each line has seven tab separated columns: PathId, FileId, JobId, LStat, MD5, volume name and InChanger.",
        ],
        examples: &[
            (".bvfs_versions jobid=0 client=bareos-fd path=/etc/ fname=hosts", "all versions of /etc/hosts"),
            (".bvfs_versions jobid=0 client=bareos-fd path=/etc/ fname=hosts copies", "including versions from copy jobs"),
        ],
    },
    Doc {
        path: ".bvfs_restore",
        long: &[
            "Creates a table in the catalog holding a restore selection: single files (fileid=), whole directories with their content (dirid=, PathIds from .bvfs_lsdirs) and hardlinks (hardlink=, pairs of JobId and FileIndex). At least one of them is required. path= names the table (b2 followed by a number); an existing table of that name is replaced.",
            "Pass the table to restore with file=?<table>; restore then skips the menu and the file tree. Drop the table afterwards with .bvfs_cleanup. The command only writes to the catalog and starts no job.",
        ],
        examples: &[
            (".bvfs_restore path=b2101 jobid=600,615,616 fileid=4711,4712", "select two files for a restore"),
            (".bvfs_restore path=b2102 jobid=600,615,616 dirid=7", "select a whole directory"),
        ],
    },
    Doc {
        path: ".volume_damaged",
        long: &[
            "Onesimus extension: the flow for a damaged Volume (tape tear, disk failure, a broken file Volume). Without yes (or with dryrun) it only reports: the jobs that have data on the Volume (a job over several Volumes is lost as a whole, the other Volumes are named), for each job whether a usable copy exists (a finished Backup or Copy job of the same original that lies on no damaged Volume and no Volume with status Error), the Full backups to start for the jobs without a copy, and the later Incremental and Differential jobs of a damaged Full.",
            "With yes the Volume is set to VolStatus Error and the damaged jobs that have a usable copy are purged; the oldest copy becomes a Backup job (UpgradeCopies), so a restore finds it. Jobs without a copy stay in the catalog. full additionally starts a Full of every job without a copy.",
        ],
        examples: &[
            (".volume_damaged volume=Vol-0007", "what a damaged Volume costs"),
            (".volume_damaged volume=Vol-0007 yes full", "set it to Error, promote the copies, start new Fulls"),
        ],
    },
    Doc {
        path: ".ovfs_status",
        long: &[
            "Onesimus extension: shows whether the PostgreSQL extension ovfs is installed and usable in the catalog database and which path answers the .ovfs commands, the extension or the equivalent plain SQL fallback in the Director. Both give the same results; the extension is faster on large catalogs. If the extension is not usable the reason is shown.",
            "refresh checks the extension again instead of showing the stored result. json prints the result as one JSON line in every API mode.",
        ],
        examples: &[
            (".ovfs_status", "extension state and restore tree path"),
            (".ovfs_status refresh json", "check again and print JSON"),
        ],
    },
    Doc {
        path: ".ovfs_latest",
        long: &[
            "Onesimus extension: determines the job chain for the newest state of a client, as restore menu item 5 selects it: the last Full backup of the fileset, the last Differential after it and all Incrementals after Full or Differential. Only backups with status T or W count, Differentials and Incrementals only if they are on an enabled volume. fileset= is needed when the client has backups of several filesets.",
            "Pass the comma separated list as jobid= to .ovfs_tree and .ovfs_ls, so the tree stays the same while browsing even if a new backup finishes. json prints the result as one JSON line.",
        ],
        examples: &[
            (".ovfs_latest client=bareos-fd", "the newest job chain of a client"),
            (".ovfs_latest client=bareos-fd fileset=LinuxAll json", "the chain of one fileset as JSON"),
        ],
    },
    Doc {
        path: ".ovfs_tree",
        long: &[
            "Onesimus extension: returns the directory tree of a job chain below a directory as a restore would bring it back: per path the version from the newest job, deleted files left out, hardlinks and delta files resolved. Every node with a catalog record carries its restore identity (jobid, fileindex). No cache is needed. The chain is given by jobid= or by client= (and fileset=) for the newest state like .ovfs_latest.",
            "root= sets the top of the tree (default /), depth= the number of levels (default 1; directories at the limit show their entry count), max_entries= the maximum number of nodes (default 100000; beyond it the command stops with a message). For large backups load the top with a small depth and expand directories with .ovfs_ls. json prints one JSON line.",
        ],
        examples: &[
            (".ovfs_tree jobid=600,615,616 root=/home/ depth=2", "two levels below /home of a job chain"),
            (".ovfs_tree client=bareos-fd depth=1 json", "the top level of the newest state as JSON"),
        ],
    },
    Doc {
        path: ".ovfs_ls",
        long: &[
            "Onesimus extension: lists the direct entries of one directory in the state of a job chain, first the files, then the subdirectories with the number of their entries, so a restore browser can load the tree directory by directory. Directories below which nothing can be restored are left out. The chain is given by jobid= or by client= (and fileset=) for the newest state; path= is required and ends with a slash.",
            "Each entry carries its restore identity (jobid, fileindex). json prints the result as one JSON line in every API mode.",
        ],
        examples: &[
            (".ovfs_ls jobid=600,615,616 path=/etc/", "the entries of /etc in a job chain"),
            (".ovfs_ls client=bareos-fd path=/home/ json", "a directory of the newest state as JSON"),
        ],
    },
    Doc {
        path: ".ovfs_versions",
        long: &[
            "Onesimus extension: lists all backed up versions of a file or directory of a client, newest first. Only versions stored on a volume from backup and archive jobs count; copies adds copy jobs. Versions from jobs the console may not see are left out. path= is the directory, name= the file; without name= the versions of the directory itself are listed.",
            "Each line has ten tab separated columns: JobId, level, job type, start time, size, modification time, FileIndex, volumes, InChanger and MD5. json prints the result as one JSON line. Unlike .bvfs_versions no PathIds are needed.",
        ],
        examples: &[
            (".ovfs_versions client=bareos-fd path=/etc/ name=hosts", "all versions of /etc/hosts"),
            (".ovfs_versions client=bareos-fd path=/etc/ name=fstab copies json", "including copy jobs, as JSON"),
        ],
    },
    Doc {
        path: "configure add",
        long: &[
            "Creates a new resource of the given type (client, jobdefs, job, storage, catalog, schedule, fileset, pool, messages, counter, profile, console, user). The resource is written to bareos-dir.d/<type>/<name>.conf and loaded into the running Director at once, without reload. Directives are written in lower case without blanks (maximumvolumejobs=1); a directive may repeat where the resource allows it (run= of a schedule). The characters @ and ; are not allowed in values.",
            "The configuration must be in directory form and the directory of the type must already exist. Adding a client also exports its file daemon resource, like configure export. Onesimus deviation: profile and user are always refused (edit them in the configuration files), and console is only accepted as name, password, one or more profile and an optional description, with profiles that grant no more than the creating console may use itself.",
        ],
        examples: &[
            ("configure add pool name=Weekly pooltype=Backup maximumvolumejobs=1 labelformat=Weekly-", "a new backup pool with one job per volume"),
            ("configure add schedule name=Nightly run=\"Full 1st sun at 23:05\" run=\"Incremental mon-sat at 23:05\"", "a schedule with two run lines"),
            ("configure add job name=BackupClient2 jobdefs=DefaultJob client=bareos-fd", "a job whose required directives come from a jobdefs"),
        ],
    },
    Doc {
        path: "configure export",
        long: &[
            "Writes the Director resource a file daemon needs to accept this Director, for the given client, to bareos-dir-export/client/<client>/bareos-fd.d/director/. Copy the file to the client into bareos-fd.d/director/. Calling it again overwrites the file with the same content. client= must name a configured client.",
        ],
        examples: &[
            ("configure export client=bareos-fd", "export the director resource for bareos-fd"),
        ],
    },
    Doc {
        path: "configure read",
        long: &[
            "Onesimus extension: shows a short summary of one client, job, pool or storage of the running configuration (for a client name, address, port and enabled; for a job type, level, client, storage, pool, fileset, priority and enabled). The resource ACLs of the console apply as with show; a resource the console may not see reads like an unknown one. For the raw resource text use .config read.",
        ],
        examples: &[
            ("configure read client=bareos-fd", "summary of a client"),
            ("configure read job=BackupClient1", "summary of a job"),
        ],
    },
    Doc {
        path: "configure update",
        long: &[
            "Onesimus extension: changes directives of an existing client, job, jobdefs, schedule or fileset in its configuration file and reloads. Only the named directives change; comments, order, indentation and all other directives stay byte for byte. It is checked like odir -t, written atomically, and a failed reload restores the old state. pool, storage and console are refused, as are directives in nested blocks (RunScript, Include, Options) and changing the name.",
            "A directive given several times replaces all its lines (the run lines of a schedule); a directive with an empty value (description=) removes it. A missing directive is added before the closing brace. The resource ACL of the console applies. The old job keys newclient, newstorage and newpool are accepted as client, storage and pool.",
        ],
        examples: &[
            ("configure update client=bareos-fd address=fd.example.org port=9112", "change the address and port of a client"),
            ("configure update job=BackupClient1 description=", "remove the description of a job"),
        ],
    },
    Doc {
        path: "configure delete",
        long: &[
            "Onesimus extension: deletes a client, job, jobdefs, schedule, fileset, pool or storage through the configuration CRUD, the same path as .config delete. The resource is read for its current checksum and deleted only if no other resource still refers to it; otherwise the referring resources are listed and nothing is deleted. The remaining configuration is checked like odir -t, the file change is atomic and a failed reload is rolled back.",
            "pool, storage and console are refused; remove such files by hand and reload. The resource ACL of the console applies.",
        ],
        examples: &[
            ("configure delete fileset=OldFileSet", "delete a fileset no job uses any more"),
            ("configure delete client=old-fd", "delete a client, refused while a job still names it"),
        ],
    },
    Doc {
        path: ".acl list",
        long: &[
            "Shows per ACL type the file entries (first those of the console, then those of its profiles), the catalog entries and the effective result, plus the load state of the catalog ACL. list is also the default when no subcommand is given. Without console= the own console is shown, or all consoles for a console that may use configure; the ACLs of another console need configure. Catalog entries that only allow what the file ACL forbids are marked as having no effect.",
        ],
        examples: &[
            (".acl list", "the ACLs of the own console"),
            (".acl list console=operator", "the file, catalog and effective ACLs of another console"),
        ],
    },
    Doc {
        path: ".acl set",
        long: &[
            "Replaces the catalog list of the console for every ACL type given. Entries are evaluated like a Bareos list: the first matching entry decides, ! forbids, regular expressions and *all* work as in the file, and a name no entry matches is forbidden. The change takes effect at once. Needs the CommandACL .acl and configure.",
            "The catalog ACL of another console may be changed freely, since it can never go beyond that console's file ACL. On the own console only a narrowing change is allowed: a first list for a type without catalog entries, or a list made from the old one only by inserting ! entries and removing entries without !.",
        ],
        examples: &[
            (".acl set console=ops clientacl=bareos-fd,!old-fd", "restrict a console to some clients"),
        ],
    },
    Doc {
        path: ".acl delete",
        long: &[
            "Removes the given entries from the catalog ACL of the console; a type key without a value removes the whole catalog list of that type, which then no longer restricts. An entry the catalog ACL does not have is an error and nothing changes. Works also for a console that is no longer configured but still has catalog entries. Needs the CommandACL .acl and configure; on the own console only removing entries without ! is allowed.",
        ],
        examples: &[
            (".acl delete console=ops commandacl=status", "remove one catalog entry"),
            (".acl delete console=ops poolacl", "remove the whole catalog pool list"),
        ],
    },
    Doc {
        path: ".acl clear",
        long: &[
            "Removes all catalog ACL entries of the console, so only its file ACL applies again. Works also for a console that is no longer configured. Needs the CommandACL .acl and configure and is refused on the own console, since it could widen the rights.",
        ],
        examples: &[
            (".acl clear console=ops", "drop all catalog restrictions of a console"),
        ],
    },
    Doc {
        path: ".acl copy",
        long: &[
            "Gives the console to= the catalog ACL lists of the console from=. Types that from= has no catalog entries for lose their entries at to=. The file ACLs of both consoles stay as they are. Needs the CommandACL .acl and configure.",
        ],
        examples: &[
            (".acl copy from=ops to=gui-anna", "apply the catalog restrictions of ops to gui-anna"),
        ],
    },
    Doc {
        path: ".config capabilities",
        long: &[
            "Shows the interface version and, per resource type, the operations the configuration CRUD offers (READ_RAW, CREATE_RAW, UPDATE_RAW, DELETE). The editable types are schedule, job, jobdefs and client. Without the CommandACL configure no types are listed.",
        ],
        examples: &[
            (".config capabilities", "the editable types and their operations"),
        ],
    },
    Doc {
        path: ".config list",
        long: &[
            "Lists the resources of one type (schedule, job, jobdefs or client) with their file, SHA-256 and whether the file holds more than one resource. Resources the console may not see by its resource ACL are left out.",
        ],
        examples: &[
            (".config list type=client", "all client resources with file and checksum"),
        ],
    },
    Doc {
        path: ".config read",
        long: &[
            "Returns the raw text of one resource byte for byte, with its file and SHA-256. The checksum is the optimistic lock for a later update or delete. A resource the console may not see gives the status DENIED.",
        ],
        examples: &[
            (".config read type=client name=bareos-fd", "the raw client text and its checksum"),
        ],
    },
    Doc {
        path: ".config create",
        long: &[
            "Creates a resource from its raw text, given base64 encoded in text64= because a console line carries no line ends. The new configuration is checked like odir -t, written atomically and reloaded, with rollback if the reload fails. With validate_only the text is only checked, nothing is written.",
            "A result status other than OK (EXISTS, INVALID, DENIED ...) is reported in the response, not as a command error. Needs the CommandACL configure.",
        ],
        examples: &[
            (".config create type=schedule text64=U2NoZWR1bGUgewogIE5hbWUgPSBOaWdodGx5CiAgUnVuID0gRnVsbCBzdW4gYXQgMjM6MDAKfQo validate_only", "check a new schedule without writing it"),
        ],
    },
    Doc {
        path: ".config update",
        long: &[
            "Replaces the raw text of a resource. sha256= must be the checksum from the last read; if the file changed since, the status is CONFLICT and nothing is written. The text is checked like odir -t, written atomically and reloaded with rollback; validate_only only checks. Needs the CommandACL configure.",
        ],
        examples: &[
            (".config update type=schedule name=Nightly sha256=5b1e9a3c text64=U2NoZWR1bGUgewogIE5hbWUgPSBOaWdodGx5CiAgUnVuID0gRnVsbCBzYXQgYXQgMjI6MDAKfQo", "replace a schedule with new text"),
        ],
    },
    Doc {
        path: ".config delete",
        long: &[
            "Deletes a resource, guarded by the checksum from the last read. If other resources still refer to it the status is REFERENCED, the referring resources are listed and nothing is deleted. validate_only only checks. Needs the CommandACL configure.",
        ],
        examples: &[
            (".config delete type=client name=old-fd sha256=5b1e9a3c validate_only", "check whether a client could be deleted"),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_grammar::{roots, validate};

    /// The node of `path` below the roots.
    fn find<'a>(roots: &'a [Node], path: &str) -> Option<&'a Node> {
        let mut words = path.split(' ');
        let first = words.next()?;
        let mut node = roots.iter().find(|n| n.word == first)?;
        for word in words {
            node = node.subs.iter().find(|s| s.word == word)?;
        }
        Some(node)
    }

    /// Every path names a node; paragraphs live either in the grammar table
    /// or here; every entry adds something.
    #[test]
    fn docs_name_existing_nodes() {
        let table: Vec<Node> = crate::command_executor::get_registry().iter()
            .filter_map(|c| crate::grammar_table::describe(c.name).map(|mut n| { n.word = c.name.to_string(); n }))
            .collect();
        let mut seen = std::collections::HashSet::new();
        for doc in DOCS {
            assert!(seen.insert(doc.path), "{}: twice", doc.path);
            assert!(find(roots(), doc.path).is_some(), "{}: no such node", doc.path);
            let inline = find(&table, doc.path).unwrap_or_else(|| panic!("{}", doc.path));
            assert!(doc.long.is_empty() || inline.long.is_empty(), "{}: paragraphs in grammar_table and here", doc.path);
            assert!(!doc.long.is_empty() || !doc.examples.is_empty(), "{}: empty", doc.path);
            for paragraph in doc.long {
                assert!(!paragraph.trim().is_empty() && !paragraph.contains('`'), "{}: {paragraph}", doc.path);
            }
        }
    }

    /// Every top-level command has DESCRIPTION paragraphs and an example.
    #[test]
    fn every_command_has_long_and_an_example() {
        let missing: Vec<&str> = roots().iter()
            .filter(|r| r.long.is_empty() || r.examples.is_empty())
            .map(|r| r.word.as_str()).collect();
        assert!(missing.is_empty(), "without long or example: {missing:?}");
    }

    /// Every example line is accepted by the grammar.
    #[test]
    fn examples_are_grammatical() {
        let roots = roots();
        let mut bad = Vec::new();
        for root in roots {
            for node in root.walk() {
                for (line, _) in &node.examples {
                    if let Err(e) = validate(roots, line) {
                        bad.push(format!("{line}: {e}"));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}
