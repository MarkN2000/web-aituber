use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(120);
const STARTUP_CONFIRMATION_TIME: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const UPDATE_LOG_FILE_NAME: &str = "update.log";
const UPDATE_LOG_ENV: &str = "WEB_AITUBER_UPDATE_LOG";

struct Args {
    parent_pid: u32,
    install_root: PathBuf,
    package_root: PathBuf,
    work_root: PathBuf,
    executable_name: String,
}

struct SwappedItem {
    name: String,
    had_original: bool,
}

fn main() {
    let values: Vec<OsString> = env::args_os().skip(1).collect();
    let log_path = values
        .get(1)
        .map(PathBuf::from)
        .map(|root| root.join(UPDATE_LOG_FILE_NAME));
    append_update_log(log_path.as_deref(), "外部アップデーターを開始しました");
    if let Err(error) = run(values, log_path.as_deref()) {
        let message = format!("アップデートに失敗しました: {error:#}");
        append_update_log(log_path.as_deref(), &message);
        eprintln!("{message}");
    } else {
        append_update_log(log_path.as_deref(), "アップデートが完了しました");
    }
    schedule_self_delete();
}

fn run(values: Vec<OsString>, log_path: Option<&Path>) -> Result<()> {
    let args = parse_args(values)?;
    let backup_root = args.work_root.join("backup");
    fs::create_dir(&backup_root).context("バックアップ用フォルダを作成できません")?;

    append_update_log(
        log_path,
        &format!("サーバープロセス {} の終了を待ちます", args.parent_pid),
    );
    wait_for_parent(args.parent_pid, PARENT_EXIT_TIMEOUT)?;
    append_update_log(log_path, "サーバープロセスの終了を確認しました");

    let items = [
        args.executable_name.as_str(),
        updater_file_name(),
        "web",
        "config.example.json",
        "README.md",
    ];
    let mut swapped = Vec::new();
    for name in items {
        append_update_log(log_path, &format!("{name}を置き換えます"));
        if let Err(error) = swap_item(
            name,
            &args.package_root,
            &args.install_root,
            &backup_root,
            &mut swapped,
        ) {
            append_update_log(log_path, "置き換えに失敗したため旧ファイルへ戻します");
            rollback(&args.install_root, &backup_root, &swapped)
                .context("更新失敗後に旧ファイルへ戻せませんでした")?;
            append_update_log(log_path, "旧ファイルへ戻しました");
            restart_previous_server(&args.install_root, &args.executable_name, log_path)
                .context("旧バージョンも再起動できませんでした")?;
            return Err(error);
        }
        append_update_log(log_path, &format!("{name}を置き換えました"));
    }

    append_update_log(log_path, "新しいサーバーを起動します");
    let mut server = match restart_server(&args.install_root, &args.executable_name) {
        Ok(server) => server,
        Err(error) => {
            append_update_log(log_path, "起動に失敗したため旧ファイルへ戻します");
            rollback(&args.install_root, &backup_root, &swapped)?;
            restart_previous_server(&args.install_root, &args.executable_name, log_path)
                .context("旧バージョンも再起動できませんでした")?;
            return Err(error);
        }
    };

    match wait_for_early_exit(&mut server, STARTUP_CONFIRMATION_TIME) {
        Ok(Some(status)) => {
            append_update_log(
                log_path,
                &format!("新しいサーバーが起動直後に終了しました: {status}"),
            );
            rollback(&args.install_root, &backup_root, &swapped)?;
            restart_previous_server(&args.install_root, &args.executable_name, log_path)
                .context("旧バージョンも再起動できませんでした")?;
            bail!("新しいサーバーが起動直後に終了したため旧バージョンへ戻しました");
        }
        Ok(None) => append_update_log(log_path, "新しいサーバーの起動を確認しました"),
        Err(error) => {
            append_update_log(
                log_path,
                "新しいサーバーの確認に失敗したため旧ファイルへ戻します",
            );
            let _ = server.kill();
            let _ = server.wait();
            rollback(&args.install_root, &backup_root, &swapped)?;
            restart_previous_server(&args.install_root, &args.executable_name, log_path)
                .context("旧バージョンも再起動できませんでした")?;
            return Err(error);
        }
    }

    if let Err(error) = fs::remove_dir_all(&args.work_root) {
        append_update_log(
            log_path,
            &format!("更新用フォルダを削除できませんでした: {error}"),
        );
        eprintln!("更新用フォルダを削除できませんでした: {error}");
    }
    Ok(())
}

fn append_update_log(path: Option<&Path>, message: &str) {
    let Some(path) = path else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{message}");
    let _ = file.flush();
}

fn parse_args(values: Vec<OsString>) -> Result<Args> {
    if values.len() != 5 {
        bail!("アップデーターの引数が不足しています");
    }
    let parent_pid = values[0]
        .to_str()
        .context("親プロセスIDが不正です")?
        .parse()
        .context("親プロセスIDが不正です")?;
    let executable_name = values[4]
        .to_str()
        .context("実行ファイル名が不正です")?
        .to_owned();
    if executable_name != executable_file_name() {
        bail!("実行ファイル名が一致しません");
    }
    Ok(Args {
        parent_pid,
        install_root: PathBuf::from(&values[1]),
        package_root: PathBuf::from(&values[2]),
        work_root: PathBuf::from(&values[3]),
        executable_name,
    })
}

fn swap_item(
    name: &str,
    package_root: &Path,
    install_root: &Path,
    backup_root: &Path,
    swapped: &mut Vec<SwappedItem>,
) -> Result<()> {
    let source = package_root.join(name);
    let destination = install_root.join(name);
    let backup = backup_root.join(name);
    if !source.exists() {
        bail!("更新ファイルに{name}がありません");
    }
    let had_original = destination.exists();
    if had_original {
        fs::rename(&destination, &backup)
            .with_context(|| format!("現在の{name}を退避できません"))?;
    }
    swapped.push(SwappedItem {
        name: name.to_owned(),
        had_original,
    });
    fs::rename(&source, &destination)
        .with_context(|| format!("新しい{name}へ置き換えられません"))?;
    Ok(())
}

fn rollback(install_root: &Path, backup_root: &Path, swapped: &[SwappedItem]) -> Result<()> {
    let mut first_error = None;
    for item in swapped.iter().rev() {
        let destination = install_root.join(&item.name);
        if let Err(error) = remove_path(&destination)
            && first_error.is_none()
        {
            first_error = Some(
                anyhow::Error::new(error).context(format!("新しい{}を取り除けません", item.name)),
            );
            continue;
        }
        if item.had_original
            && let Err(error) = fs::rename(backup_root.join(&item.name), &destination)
            && first_error.is_none()
        {
            first_error = Some(error.into());
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

fn remove_path(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn restart_server(install_root: &Path, executable_name: &str) -> Result<Child> {
    let mut command = Command::new(install_root.join(executable_name));
    command
        .current_dir(install_root)
        .env(UPDATE_LOG_ENV, install_root.join(UPDATE_LOG_FILE_NAME));
    configure_hidden_process(&mut command);
    command.spawn().context("サーバーを再起動できません")
}

fn restart_previous_server(
    install_root: &Path,
    executable_name: &str,
    log_path: Option<&Path>,
) -> Result<()> {
    append_update_log(log_path, "旧バージョンを再起動します");
    let mut server = restart_server(install_root, executable_name)?;
    if let Some(status) = wait_for_early_exit(&mut server, STARTUP_CONFIRMATION_TIME)? {
        bail!("旧バージョンが起動直後に終了しました: {status}");
    }
    append_update_log(log_path, "旧バージョンの起動を確認しました");
    Ok(())
}

fn wait_for_early_exit(child: &mut Child, duration: Duration) -> Result<Option<ExitStatus>> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().context("サーバーの状態を確認できません")?
        {
            return Ok(Some(status));
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(None)
}

#[cfg(unix)]
fn wait_for_parent(parent_pid: u32, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let result = unsafe { libc::kill(parent_pid as i32, 0) };
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            if error.raw_os_error() != Some(libc::EPERM) {
                return Err(error).context("サーバープロセスの終了を確認できません");
            }
        }
        if Instant::now() >= deadline {
            bail!("サーバープロセスが終了しませんでした");
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(windows)]
fn wait_for_parent(parent_pid: u32, timeout: Duration) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, parent_pid) };
    if handle.is_null() {
        return Ok(());
    }
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    let wait_result = unsafe { WaitForSingleObject(handle, milliseconds) };
    unsafe { CloseHandle(handle) };
    match wait_result {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => bail!("サーバープロセスが終了しませんでした"),
        _ => bail!("サーバープロセスの終了を確認できません"),
    }
}

#[cfg(windows)]
fn configure_hidden_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_hidden_process(_command: &mut Command) {}

#[cfg(unix)]
fn schedule_self_delete() {
    if let Ok(path) = env::current_exe() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(windows)]
fn schedule_self_delete() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
    if let Ok(path) = env::current_exe() {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT);
        }
    }
}

fn executable_file_name() -> &'static str {
    if cfg!(windows) {
        "web-aituber.exe"
    } else {
        "web-aituber"
    }
}

fn updater_file_name() -> &'static str {
    if cfg!(windows) {
        "web-aituber-updater.exe"
    } else {
        "web-aituber-updater"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unexpected_executable_name() {
        let values = vec![
            OsString::from("1"),
            OsString::from("install"),
            OsString::from("package"),
            OsString::from("work"),
            OsString::from("other.exe"),
        ];
        assert!(parse_args(values).is_err());
    }

    #[test]
    fn swapped_file_can_be_rolled_back() {
        let root =
            env::temp_dir().join(format!("web-aituber-updater-test-{}", uuid::Uuid::new_v4()));
        let install = root.join("install");
        let package = root.join("package");
        let backup = root.join("backup");
        fs::create_dir_all(&install).unwrap();
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(install.join("README.md"), b"old").unwrap();
        fs::write(package.join("README.md"), b"new").unwrap();

        let mut swapped = Vec::new();
        swap_item("README.md", &package, &install, &backup, &mut swapped).unwrap();
        assert_eq!(fs::read(install.join("README.md")).unwrap(), b"new");

        rollback(&install, &backup, &swapped).unwrap();
        assert_eq!(fs::read(install.join("README.md")).unwrap(), b"old");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_log_appends_progress() {
        let root =
            env::temp_dir().join(format!("web-aituber-updater-log-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join(UPDATE_LOG_FILE_NAME);

        append_update_log(Some(&path), "開始");
        append_update_log(Some(&path), "完了");

        assert_eq!(fs::read_to_string(&path).unwrap(), "開始\n完了\n");
        fs::remove_dir_all(root).unwrap();
    }
}
