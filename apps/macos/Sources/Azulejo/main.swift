import AppKit

/// What the launcher passed in. A plain launch sets none of these and gets the
/// real thing: the user's config, saved state, herdr session and API socket.
/// `apps/macos/dev.sh` sets them for the isolated dev profile.
struct LaunchEnvironment {
    /// `AZULEJO_PROFILE`: `dev` refuses herdr's default session.
    var profile: String?
    /// `AZULEJO_CORE_OPTIONS`: JSON for `azj_core_new_with_options`.
    var coreOptions: String?
    /// `AZULEJO_PIDFILE`: where to write our pid (dev.sh uses it to stop us).
    var pidfile: String?
    /// `AZULEJO_BACKGROUND=1`: never activate the app or raise its windows.
    var background: Bool
    /// `AZULEJO_AUTOMATION`: an automation script to run (see Automation.swift).
    var automation: String?
    /// `AZULEJO_AUTOMATION_OUT`: where automation writes its log and shots.
    var automationOut: String
    /// `AZULEJO_VERBOSE=1` (or any dev/automation run): log keys and failures to stderr.
    var verbose: Bool

    static func current() -> LaunchEnvironment {
        let env = ProcessInfo.processInfo.environment
        let automation = env["AZULEJO_AUTOMATION"].flatMap { $0.isEmpty ? nil : $0 }
        let profile = env["AZULEJO_PROFILE"]
        return LaunchEnvironment(
            profile: profile,
            coreOptions: env["AZULEJO_CORE_OPTIONS"].flatMap { $0.isEmpty ? nil : $0 },
            pidfile: env["AZULEJO_PIDFILE"].flatMap { $0.isEmpty ? nil : $0 },
            background: env["AZULEJO_BACKGROUND"] == "1" || automation != nil,
            automation: automation,
            automationOut: env["AZULEJO_AUTOMATION_OUT"] ?? FileManager.default.currentDirectoryPath,
            verbose: env["AZULEJO_VERBOSE"] == "1" || profile == "dev" || automation != nil)
    }

    /// The dev profile must name its own herdr session (or none). Checked
    /// before the core boots, so it never touches the default session.
    func devProfileProblem() -> String? {
        guard profile == "dev" else { return nil }
        guard let coreOptions,
              let data = coreOptions.data(using: .utf8),
              let options = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return "the dev profile needs AZULEJO_CORE_OPTIONS" }
        if options["herdr"] as? Bool == false || options["herdr_session"] is NSNull { return nil }
        guard let session = options["herdr_session"] as? String else {
            return "the dev profile must set herdr_session (it would fall back to the config's)"
        }
        if session.isEmpty || session == "default" {
            return "the dev profile refuses herdr's default session"
        }
        return nil
    }
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("azulejo: \(message)\n".utf8))
    exit(2)
}

MainActor.assumeIsolated { Startup.mark("main") }
let launch = LaunchEnvironment.current()
if let problem = launch.devProfileProblem() { fail(problem) }

let app = NSApplication.shared
app.setActivationPolicy(launch.automation != nil ? .accessory : .regular)

let delegate: AppController = MainActor.assumeIsolated {
    guard let core = Core(options: launch.coreOptions) else {
        fail("the core didn't start (bad AZULEJO_CORE_OPTIONS, or the API socket is taken)")
    }
    // Belt and braces: whatever the options said, a dev run must not be on the
    // default herdr session.
    if launch.profile == "dev", core.info()["herdr_session"] as? String == "default" {
        core.shutdown()
        fail("dev profile ended up on herdr's default session; refusing")
    }
    Startup.mark("core")
    return AppController(core: core, env: launch)
}
app.delegate = delegate
withExtendedLifetime(delegate) { app.run() }
