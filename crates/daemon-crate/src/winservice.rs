//! Windows-Dienst: Anmeldung beim Service Control Manager (SCM).
//!
//! Zweck: Unter Windows startet der SCM den Daemon als Dienst und erwartet,
//! dass er sich binnen 30 s mit `StartServiceCtrlDispatcher` meldet, seinen
//! Zustand mit `SetServiceStatus` berichtet und auf „Beenden“ (Stop) und
//! das Herunterfahren des Rechners (Shutdown) reagiert; sonst bricht der
//! Start mit Fehler 1053 ab. Unix kennt das nicht: dort ersetzt dieses
//! Modul [`crate::background`] (Hintergrund-Start) und die Abschaltsignale
//! aus [`crate::signals`].
//!
//! Ablauf: [`run`] ruft `StartServiceCtrlDispatcher`. Läuft der Prozess
//! nicht unter dem SCM (Aufruf aus einer Konsole), kehrt der Aufruf sofort
//! mit `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` (1063) zurück; [`run`]
//! liefert dann [`Dispatch::NotAService`], und der Daemon läuft im
//! Vordergrund weiter (Strg-C beendet ihn). Dadurch braucht der Dienst
//! kein eigenes Argument: der MSI-Installer registriert ofd mit
//! `-f -c … --log-file …`, und genau diese Zeile funktioniert in der
//! Konsole wie unter dem SCM.
//!
//! Unter dem SCM: Dispatcher-Thread → Dienst-Hauptfunktion → meldet
//! `START_PENDING`, ruft den Daemon mit einem [`Service`]-Griff. Der Daemon
//! meldet sich mit [`Service::running`] betriebsbereit und wartet in
//! [`Service::wait_for_stop`]; Stop/Shutdown des SCM wecken ihn
//! (`STOP_PENDING`). Kehrt der Daemon zurück, meldet die Hauptfunktion
//! `STOPPED` mit seinem Exit-Code (≠ 0 als dienstspezifischer Fehler).
//!
//! Bareos-Gegenstück: `core/src/win32/generic/service.cc`
//! (`bareosServiceMain`, `serviceStartCallback`, `serviceControlCallback`,
//! `ReportStatus`, `bareosWorkerThread`) und `win32/generic/main.cc`
//! (`WinMain`). Bareos startet den Dienst über das Argument `/service`
//! und meldet `START_PENDING` mit 45 s Wartezeit, dann `RUNNING`; es nimmt
//! nur `SERVICE_CONTROL_STOP` an. Hier zusätzlich `SHUTDOWN`, damit
//! laufende Jobs beim Herunterfahren sauber enden. `/install` und
//! `/remove` (`installService`, `removeService`) gibt es nicht: den Dienst
//! registriert der MSI-Installer.

use std::ffi::OsString;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle};
use windows_service::{define_windows_service, service_dispatcher};

/// `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT`: not started by the SCM.
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;

/// Wait hint while starting (Bareos: 45000 ms in `serviceStartCallback`).
const START_WAIT_HINT: Duration = Duration::from_secs(45);

/// Wait hint while stopping: the daemon joins its threads.
const STOP_WAIT_HINT: Duration = Duration::from_secs(30);

/// The daemon's body under the SCM: gets the [`Service`] handle, returns
/// the exit code.
type App = Box<dyn FnOnce(&Service) -> i32 + Send>;

/// Name of the service (set once by [`run`]).
static NAME: OnceLock<&'static str> = OnceLock::new();

/// The daemon's body, taken by the service main function.
static APP: Mutex<Option<App>> = Mutex::new(None);

/// Result of [`run`].
#[derive(Debug)]
pub enum Dispatch {
    /// The service ran under the SCM and has stopped.
    Ran,
    /// The process was not started by the SCM (console start); the caller
    /// runs the daemon in the foreground itself.
    NotAService,
    /// `StartServiceCtrlDispatcher` failed for another reason.
    Failed(String),
}

/// Runs `app` as service `name` if the SCM started this process.
///
/// Returns [`Dispatch::NotAService`] at once for a console start; then
/// `app` was not called.
pub fn run<F>(name: &'static str, app: F) -> Dispatch
where
    F: FnOnce(&Service) -> i32 + Send + 'static,
{
    let _ = NAME.set(name);
    *APP.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(app));
    match service_dispatcher::start(name, ffi_service_main) {
        Ok(()) => Dispatch::Ran,
        Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) => {
            APP.lock().unwrap_or_else(|e| e.into_inner()).take();
            Dispatch::NotAService
        }
        Err(e) => Dispatch::Failed(e.to_string()),
    }
}

define_windows_service!(ffi_service_main, service_main);

/// `serviceStartCallback` + `bareosWorkerThread`: registers the control
/// handler, reports `START_PENDING`, runs the daemon and reports `STOPPED`.
fn service_main(_arguments: Vec<OsString>) {
    let name = NAME.get().copied().unwrap_or("ofd");
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let (tx, rx) = mpsc::channel();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let handle = match service_control_handler::register(name, handler) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("RegisterServiceCtrlHandler failed: {e}");
            return;
        }
    };
    let service = Service { handle, stop: Mutex::new(rx), checkpoint: Mutex::new(1) };
    service.report(ServiceState::StartPending, START_WAIT_HINT, 0);
    let code = app(&service);
    service.report(ServiceState::Stopped, Duration::ZERO, code);
}

/// The daemon's handle on the SCM while it runs as a service.
pub struct Service {
    /// Status handle of `RegisterServiceCtrlHandlerEx`.
    handle: ServiceStatusHandle,
    /// Woken by Stop/Shutdown from the control handler.
    stop: Mutex<mpsc::Receiver<()>>,
    /// `dwCheckPoint` of the pending states (`ReportStatus`).
    checkpoint: Mutex<u32>,
}

impl Service {
    /// Reports `SERVICE_RUNNING`: the daemon accepts connections.
    pub fn running(&self) {
        self.report(ServiceState::Running, Duration::ZERO, 0);
    }

    /// Blocks until the SCM sends Stop or Shutdown, then reports
    /// `SERVICE_STOP_PENDING`.
    pub fn wait_for_stop(&self) {
        let _ = self.stop.lock().unwrap_or_else(|e| e.into_inner()).recv();
        tracing::info!("service stop requested");
        self.report(ServiceState::StopPending, STOP_WAIT_HINT, 0);
    }

    /// `ReportStatus`: no controls while starting, Stop/Shutdown otherwise;
    /// the checkpoint counts up in the pending states and is 0 otherwise.
    fn report(&self, state: ServiceState, wait_hint: Duration, code: i32) {
        let controls_accepted = match state {
            ServiceState::StartPending | ServiceState::Stopped => ServiceControlAccept::empty(),
            _ => ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        };
        let checkpoint = match state {
            ServiceState::StartPending | ServiceState::StopPending => {
                let mut c = self.checkpoint.lock().unwrap_or_else(|e| e.into_inner());
                *c += 1;
                *c
            }
            _ => 0,
        };
        let exit_code = if code == 0 { ServiceExitCode::Win32(0) } else { ServiceExitCode::ServiceSpecific(code as u32) };
        let status = ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint,
            wait_hint,
            process_id: None,
        };
        if let Err(e) = self.handle.set_service_status(status) {
            tracing::error!("SetServiceStatus failed: {e}");
        }
    }
}
