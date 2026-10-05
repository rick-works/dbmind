//! 在线更新：检查 GitHub 最新 Release，与本地版本比对；可选下载安装包并拉起安装器。
//!
//! - `GET  /update/check` —— 返回 { current, latest, hasNew, name, notes, url }
//! - `POST /update/apply` —— 下载最新 NSIS 安装包到 %TEMP%，启动安装向导（用户手点完成）
//!
//! 仓库私有时 GitHub API 匿名访问 404 —— 自动带上
//! %USERPROFILE%\.dbmind\github-token 里的私人令牌（推送用的同一个文件）；
//! 公开仓库没有该文件则匿名访问。

use axum::Json;
use dbmind_core::{DbMindError, ErrorCode};
use serde_json::{json, Value};

const RELEASE_LATEST: &str = "https://api.github.com/repos/rick-works/dbmind/releases/latest";

/// GitHub 私人令牌（可选）：私有仓库的 API 需要它；文件不存在则匿名访问。
fn github_token() -> Option<String> {
    let base = std::env::var("USERPROFILE").ok()?;
    let token = std::fs::read_to_string(
        std::path::Path::new(&base).join(".dbmind").join("github-token"),
    )
    .ok()?;
    let token = token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// 拉取最新发行版信息（阻塞 HTTP，放 blocking 线程池）。
/// GitHub 的 releases/latest 直接返回单个最新发行版（不需要像 Gitee 那样列表取第一个）。
async fn fetch_latest() -> dbmind_core::Result<Value> {
    crate::api::blocking(|| {
        let mut request = ureq::get(RELEASE_LATEST)
            .set("User-Agent", "dbmind-updater")
            .timeout(std::time::Duration::from_secs(10));
        if let Some(tk) = github_token() {
            request = request.set("Authorization", &format!("Bearer {tk}"));
        }
        let response = request.call().map_err(|err| {
            DbMindError::new(ErrorCode::Internal, format!("连接更新服务器失败：{err}"))
        })?;
        let mut bytes: Vec<u8> = Vec::new();
        std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)?;
        serde_json::from_slice::<Value>(&bytes)
            .map_err(|err| DbMindError::new(ErrorCode::Internal, format!("解析发行版信息失败：{err}")))
    })
    .await
    .map_err(|x| DbMindError::new(ErrorCode::Internal, x.message.clone()))
}

/// 简易 semver 比较：a > b（按数字段逐段比，段数不足补 0）。
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(a), parse(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// 附件得分：NSIS 安装包 > MSI > 绿色版 zip > 其他。
fn asset_score(name: &str) -> u8 {
    if name.ends_with("-setup.exe") {
        3
    } else if name.ends_with(".msi") {
        2
    } else if name.ends_with(".zip") {
        1
    } else {
        0
    }
}

/// `GET /update/check` —— 本地版本 vs GitHub 最新发行版。
pub async fn check() -> Json<Value> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    match fetch_latest().await {
        Ok(v) => {
            let latest = v["tag_name"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches('v')
                .to_string();
            let has_new = !latest.is_empty() && version_gt(&latest, &current);
            // 下载地址：优先 NSIS 安装包，其次 MSI / 绿色版 zip
            let mut download = String::new();
            let mut prefer = 0u8;
            if let Some(assets) = v["assets"].as_array() {
                for asset in assets {
                    let name = asset["name"].as_str().unwrap_or("");
                    let url = asset["browser_download_url"].as_str().unwrap_or("");
                    let score = asset_score(name);
                    if !url.is_empty() && score > prefer {
                        prefer = score;
                        download = url.to_string();
                    }
                }
            }
            Json(json!({
                "success": true,
                "current": current,
                "latest": latest,
                "hasNew": has_new,
                "name": v["name"].as_str().unwrap_or(""),
                "notes": v["body"].as_str().unwrap_or(""),
                "publishedAt": v["published_at"].as_str().unwrap_or(""),
                "download": download,
                "releaseUrl": v["html_url"].as_str().unwrap_or(""),
            }))
        }
        Err(err) => Json(json!({
            "success": false,
            "current": current,
            "message": err.message,
        })),
    }
}

/// `POST /update/apply` —— 下载最新安装包到临时目录并拉起安装向导。
///
/// 安装包优先 `-setup.exe`（NSIS），其次 `.msi`。下载阻塞（几十 MB），前端转圈等待；
/// 完成后 DETACHED 启动安装器，用户点完向导重启应用即为新版。
pub async fn apply() -> Result<Json<Value>, axum::response::Response> {
    let v = match fetch_latest().await {
        Ok(v) => v,
        Err(err) => {
            return Ok(Json(json!({ "success": false, "message": err.message })));
        }
    };
    let tag = v["tag_name"].as_str().unwrap_or("latest").to_string();
    let mut target: Option<(String, String)> = None; // (url, name)
    if let Some(assets) = v["assets"].as_array() {
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");
            let url = asset["browser_download_url"].as_str().unwrap_or("");
            if url.is_empty() {
                continue;
            }
            let score = asset_score(name);
            if score >= 2 && target.as_ref().map(|(_, n)| asset_score(n)).unwrap_or(0) < score {
                target = Some((url.to_string(), name.to_string()));
            }
        }
    }
    let Some((url, name)) = target else {
        return Ok(Json(json!({
            "success": false,
            "message": "最新发行版没有可执行的安装包（只有源码包）",
        })));
    };

    // 下载到 %TEMP%\dbmind-update\
    let dir = std::env::temp_dir().join("dbmind-update");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Ok(Json(json!({ "success": false, "message": format!("创建临时目录失败：{e}") })));
    }
    let dest = dir.join(&name);
    let dest2 = dest.clone();
    let url2 = url.clone();
    let dl = crate::api::blocking(move || {
        let mut request = ureq::get(&url2)
            .set("User-Agent", "dbmind-updater")
            .timeout(std::time::Duration::from_secs(600));
        if let Some(tk) = github_token() {
            request = request.set("Authorization", &format!("Bearer {tk}"));
        }
        let response = request.call().map_err(|err| {
            DbMindError::new(ErrorCode::Internal, format!("下载安装包失败：{err}"))
        })?;
        let mut file = std::fs::File::create(&dest2)
            .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("写安装包失败：{e}")))?;
        let mut reader = response.into_reader();
        std::io::copy(&mut reader, &mut file)
            .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("下载中断：{e}")))?;
        Ok(())
    })
    .await
    .map_err(|x| DbMindError::new(ErrorCode::Internal, x.message.clone()));
    if let Err(e) = dl {
        return Ok(Json(json!({ "success": false, "message": e.message })));
    }

    // 拉起安装器：detached，不等它退出
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Err(e) = std::process::Command::new("cmd")
            .args(["/C", "start", "", &dest.to_string_lossy()])
            .creation_flags(0x0000_0008) // DETACHED_PROCESS
            .spawn()
        {
            return Ok(Json(json!({ "success": false, "message": format!("启动安装器失败：{e}") })));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&dest).spawn();
    }

    Ok(Json(json!({
        "success": true,
        "installer": dest.to_string_lossy(),
        "version": tag.trim_start_matches('v'),
        "message": "安装向导已启动，按提示完成后请关闭本应用并重新打开新版",
    })))
}