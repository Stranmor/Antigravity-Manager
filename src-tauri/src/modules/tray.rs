use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Manager, Runtime, Emitter, Listener,
};
use crate::modules;

pub fn create_tray<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    // 1. 加载配置获取语言设置
    let config = modules::load_app_config().unwrap_or_default();
    let texts = modules::i18n::get_tray_texts(&config.language);
    
    // 2. 加载图标（macOS 使用 Template Image）
    let icon_bytes = include_bytes!("../../icons/tray-icon.png");
    let img = image::load_from_memory(icon_bytes)
        .map_err(|e| tauri::Error::Io(std::io::Error::other(e.to_string())))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    let icon = Image::new_owned(img.into_raw(), width, height);

    // 3. 定义菜单项（使用翻译文本）
    // 状态区
    let loading_text = format!("{}: ...", texts.current);
    let quota_text = format!("{}: --", texts.quota);
    let info_user = MenuItem::with_id(app, "info_user", &loading_text, false, None::<&str>)?;
    let info_quota = MenuItem::with_id(app, "info_quota", &quota_text, false, None::<&str>)?;

    // 快捷操作区
    let switch_next = MenuItem::with_id(app, "switch_next", &texts.switch_next, true, None::<&str>)?;
    let refresh_curr = MenuItem::with_id(app, "refresh_curr", &texts.refresh_current, true, None::<&str>)?;
    
    // 系统功能
    let show_i = MenuItem::with_id(app, "show", &texts.show_window, true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", &texts.quit, true, None::<&str>)?;
    
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;

    // 4. 构建菜单
    let menu = Menu::with_items(app, &[
        &info_user,
        &info_quota,
        &sep1,
        &switch_next,
        &refresh_curr,
        &sep2,
        &show_i,
        &sep3,
        &quit_i,
    ])?;

    // 4. 构建托盘
    let _ = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon(icon)
        .on_menu_event(move |app, event| {
            let app_handle = app.clone();
            match event.id().as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        #[cfg(target_os = "macos")]
                        app.set_activation_policy(tauri::ActivationPolicy::Regular).unwrap_or(());
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                "refresh_curr" => {
                    // 异步执行刷新
                    tauri::async_runtime::spawn(async move {
                        if let Ok(Some(account_id)) = modules::get_current_account_id() {
                             // 通知前端开始
                             let _ = app_handle.emit("tray://refresh-current", ());
                             
                             // 执行刷新逻辑
                             if let Ok(mut account) = modules::load_account(&account_id) {
                                 // 使用 modules::account 中的共享逻辑
                                 match modules::account::fetch_quota_with_retry(&mut account).await {
                                     Ok(quota) => {
                                         // 保存
                                         let _ = modules::update_account_quota(&account.id, quota);
                                         // 更新托盘展示
                                         update_tray_menus(&app_handle);
                                     },
                                     Err(e) => {
                                         // 错误处理，可能只记录日志
                                          modules::logger::log_error(&format!("托盘刷新失败: {e}"));
                                     }
                                 }
                             }
                        }
                    });
                }
                "switch_next" => {
                    tauri::async_runtime::spawn(async move {
                         // 1. 获取所有账号
                         if let Ok(accounts) = modules::list_accounts() {
                             if accounts.is_empty() { return; }
                             
                             let current_id = modules::get_current_account_id().unwrap_or(None);
                             let next_account = if let Some(curr) = current_id {
                                 let idx = accounts.iter().position(|a| a.id == curr).unwrap_or(0);
                                 let next_idx = (idx + 1) % accounts.len();
                                 &accounts[next_idx]
                             } else {
                                 &accounts[0]
                             };
                             
                             // 2. 切换
                             if modules::switch_account(&next_account.id).await.is_ok() {
                                 // 3. 通知前端
                                 let _ = app_handle.emit("tray://account-switched", next_account.id.clone());
                                 // 4. 更新托盘
                                 update_tray_menus(&app_handle);
                             }
                         }
                    });
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
               let app = tray.app_handle();
               if let Some(window) = app.get_webview_window("main") {
                   let _ = window.show();
                   let _ = window.set_focus();
                   #[cfg(target_os = "macos")]
                   app.set_activation_policy(tauri::ActivationPolicy::Regular).unwrap_or(());
               }
            }
        })
        .build(app)?;

    // 初始化时更新一次状态
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        update_tray_menus(&handle);
    });

    // 监听配置变更事件
    let handle = app.clone();
    app.listen("config://updated", move |_event| {
        modules::logger::log_info("配置已更新，刷新托盘菜单");
        update_tray_menus(&handle);
    });

    Ok(())
}

/// 更新托盘菜单的辅助函数
pub fn update_tray_menus<R: Runtime>(app: &tauri::AppHandle<R>) {
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
         // 读取配置获取语言
         let config = modules::load_app_config().unwrap_or_default();
         let texts = modules::i18n::get_tray_texts(&config.language);
         
         // 获取当前账号信息
         let current = modules::get_current_account_id().unwrap_or(None);
         
         let mut menu_lines = Vec::new();
         let mut user_text = format!("{}: {}", texts.current, texts.no_account);

         if let Some(id) = current {
             if let Ok(account) = modules::load_account(&id) {
                 user_text = format!("{}: {}", texts.current, account.email);
                 
                 if let Some(q) = account.quota {
                     if q.is_forbidden {
                         menu_lines.push(format!("🚫 {}", texts.forbidden));
                     } else {
                         // 提取 3 个指定模型
                         let mut gemini_high = 0;
                         let mut gemini_image = 0;
                         let mut claude = 0;
                         
                         // 使用严格匹配，与前端一致
                         for m in q.models {
                             let name = m.name.to_lowercase();
                             if name == "gemini-3-pro-high" { gemini_high = m.percentage; }
                             if name == "gemini-3-pro-image" { gemini_image = m.percentage; }
                             if name == "claude-sonnet-4-5" { claude = m.percentage; }
                         }
                         
                         menu_lines.push(format!("Gemini High: {gemini_high}%"));
                         menu_lines.push(format!("Gemini Image: {gemini_image}%"));
                         menu_lines.push(format!("Claude 4.5: {claude}%"));
                     }
                 } else {
                     menu_lines.push(texts.unknown_quota.clone());
                 }
             } else {
                 user_text = format!("{}: Error", texts.current);
                 menu_lines.push(format!("{}: --", texts.quota));
             }
         } else {
             menu_lines.push(texts.unknown_quota.clone());
         };

         // 重新构建菜单项
         let info_user = MenuItem::with_id(&app_clone, "info_user", &user_text, false, None::<&str>);
         
         // 动态创建额度项
         let mut quota_items = Vec::new();
         for (i, line) in menu_lines.iter().enumerate() {
             let item = MenuItem::with_id(&app_clone, format!("info_quota_{i}"), line, false, None::<&str>);
             if let Ok(item) = item {
                 quota_items.push(item);
             }
         }
         
         let switch_next = MenuItem::with_id(&app_clone, "switch_next", &texts.switch_next, true, None::<&str>);
         let refresh_curr = MenuItem::with_id(&app_clone, "refresh_curr", &texts.refresh_current, true, None::<&str>);
         
         let show_i = MenuItem::with_id(&app_clone, "show", &texts.show_window, true, None::<&str>);
         let quit_i = MenuItem::with_id(&app_clone, "quit", &texts.quit, true, None::<&str>);
         
         if let (Ok(i_u), Ok(s_n), Ok(r_c), Ok(s), Ok(q)) = (info_user, switch_next, refresh_curr, show_i, quit_i) {
             let sep1 = PredefinedMenuItem::separator(&app_clone).ok();
             let sep2 = PredefinedMenuItem::separator(&app_clone).ok();
             let sep3 = PredefinedMenuItem::separator(&app_clone).ok();
             
             let mut items: Vec<&dyn tauri::menu::IsMenuItem<R>> = vec![&i_u];
             // 添加动态的额度项
             for item in &quota_items {
                 items.push(item);
             }
             
             if let Some(ref s) = sep1 { items.push(s); }
             items.push(&s_n);
             items.push(&r_c);
             if let Some(ref s) = sep2 { items.push(s); }
             items.push(&s);
             if let Some(ref s) = sep3 { items.push(s); }
             items.push(&q);
             
             if let Ok(menu) = Menu::with_items(&app_clone, &items) {
                 if let Some(tray) = app_clone.tray_by_id("main") {
                     let _ = tray.set_menu(Some(menu));
                 }
             }
         }
    });
}
