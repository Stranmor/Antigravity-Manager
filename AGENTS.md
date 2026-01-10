# Antigravity Manager - Agent Notes

## Дата: 2026-01-10

---

## 🚀 МИГРАЦИЯ НА SLINT IN PROGRESS

### Фаза 1: Extract Core ✅ DONE

**Создана структура:**
```
Antigravity-Manager/
├── Cargo.toml                 # Workspace root
├── crates/
│   └── antigravity-core/      # ✅ Shared business logic
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── error.rs
│           ├── models/        # Account, Token, Quota, Config
│           ├── modules/       # Logger (stub)
│           ├── proxy/         # Config types
│           └── utils/         # HTTP client
├── src-slint/                 # ✅ Slint native UI
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/
│       ├── main.rs
│       └── ui/
│           ├── app.slint      # Main window
│           ├── dashboard.slint
│           └── components/
│               ├── theme.slint
│               ├── sidebar.slint
│               └── stats-card.slint
└── src-tauri/                 # Legacy (for upstream sync)
```

### Верификация

- ✅ `antigravity-core` компилируется
- ✅ `antigravity-desktop` (Slint) компилируется
- ✅ Приложение запускается и отображает UI

### Следующие шаги

- [ ] Фаза 2: Портировать остальные modules (account, oauth, quota)
- [ ] Фаза 3: Портировать proxy handlers
- [ ] Фаза 4: Подключить backend к Slint UI callbacks
- [ ] Фаза 5: Accounts page
- [ ] Фаза 6: Settings page
- [ ] Фаза 7: API Proxy page
- [ ] Фаза 8: Monitor page
- [ ] Фаза 9: System tray integration
- [ ] Фаза 10: CI/CD для Slint builds

---

## Upstream Sync Strategy

```bash
# Когда upstream обновляется:
git fetch upstream
git merge upstream/main

# Конфликты только в:
# - package.json (игнорируем)
# - index.html (игнорируем)  
# - src/ (deprecated, игнорируем)

# Чистый merge:
# - src-tauri/src/proxy/**  ← critical updates
# - src-tauri/src/modules/** ← business logic
```

---

## Версия

- **v3.3.20** (upstream sync)
- **Slint UI**: In Development
- Workspace: `Cargo.toml` (root)

---

## Сервис (Legacy Tauri)

```
systemctl --user status antigravity-manager.service
● antigravity-manager.service - Antigravity Manager Proxy
   Active: active (running)
   Endpoint: http://127.0.0.1:8045
```
