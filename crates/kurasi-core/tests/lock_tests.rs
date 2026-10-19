mod common;

use std::path::Path;
use std::time::Duration;

use kurasi_core::lock::{atomic_write_text, WriteLock};

#[test]
fn lock_is_released_on_drop_and_reacquired() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = WriteLock::new(tmp.path().join("write.lock"));
    {
        let _g = lock.acquire().unwrap();
    }
    let _g = lock.acquire().unwrap();
}

#[test]
fn another_handle_on_the_same_file_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("write.lock");
    let first = WriteLock::new(&path);
    let held = first.acquire().unwrap();
    // A second lock object stands for another process: it opens its own handle and waits.
    let second = WriteLock::with_timeout(&path, Duration::from_millis(300));
    let err = second.acquire().err().expect("the second handle must wait and give up");
    assert_eq!(err.status(), 409);
    drop(held);
    let _again = second.acquire().unwrap();
}

#[test]
fn python_msvcrt_lock_and_rust_lock_exclude_each_other() {
    let python = Path::new(r"D:\PythonScripts\Kurasi\.venv\Scripts\python.exe");
    if !python.exists() {
        eprintln!("legacy venv missing, skipping the cross-implementation lock test");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("write.lock");
    let flag = tmp.path().join("held");
    let code = format!(
        "import msvcrt,os,time\nfd=os.open({path:?},os.O_RDWR|os.O_CREAT)\nmsvcrt.locking(fd,msvcrt.LK_NBLCK,1)\nopen({flag:?},'w').close()\ntime.sleep(3)\n",
        path = path.to_string_lossy(),
        flag = flag.to_string_lossy()
    );
    let mut child = std::process::Command::new(python).arg("-I").arg("-c").arg(&code).spawn().expect("python starts");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !flag.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(flag.exists(), "python never took the lock");
    let lock = WriteLock::with_timeout(&path, Duration::from_millis(500));
    assert!(lock.acquire().is_err(), "the Rust lock must wait while Python holds msvcrt.locking");
    child.wait().unwrap();
    let lock = WriteLock::with_timeout(&path, Duration::from_secs(5));
    let _g = lock.acquire().unwrap();
}

#[test]
fn atomic_write_leaves_no_temp() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("sub").join("file.json");
    atomic_write_text(&target, "one").unwrap();
    atomic_write_text(&target, "two").unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "two");
    let names: Vec<String> = std::fs::read_dir(target.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["file.json"]);
}
