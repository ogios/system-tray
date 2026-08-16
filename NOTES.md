# Notes

## Electron/Chromium 托盘项不显示 (2026-08-16)

### 现象

运行 example 时日志报错,electron 应用 (netease-cloud-music-web-player) 的托盘项不显示:

```
ERROR system_tray::client: Error fetching properties from :1.153/StatusNotifierItem: Failed("error occurred in GetAll")
ERROR system_tray::client: zbus fdo error
```

其他应用 (nm-applet, fcitx, blueman 等) 正常显示。

### 根因

1. **该应用使用的 Electron/Chromium SNI 实现只响应 well-known 名称**: 应用注册时用 `org.freedesktop.StatusNotifierItem-{pid}-{n}` 这类 well-known 名称,但它的 DBus 实现只响应发往该名称的调用,发往 unique 名称 (`:1.153`) 会返回 `error occurred in GetAll`。这是对当前应用的实测结论,不代表所有 Electron/Chromium 版本都有相同行为。
   - 验证: `busctl call org.freedesktop.StatusNotifierItem-250182-1 /StatusNotifierItem org.freedesktop.DBus.Properties GetAll s org.kde.StatusNotifierItem` 成功;`busctl call :1.153 ...` 失败。
   - quickshell 保留 well-known 名称所以正常;本库 watcher 的 `parse_service` 用 `GetNameOwner` 把 well-known 转成 unique 名称,导致 GetAll 失败。

2. **单个 item 失败会杀死整个 item 流**: `handle_item` 出错时 `break` 退出循环,后续所有新注册的托盘项被永久丢弃。

### 修复

| 文件 | 修改 |
|---|---|
| `src/dbus/status_notifier_watcher.rs` | 分离服务的访问地址与生命周期 owner: `parse_service` 保留 well-known 名称用于注册/广播和后续调用,同时用 `GetNameOwner` 验证并记录注册时的 unique owner |
| `src/dbus/status_notifier_watcher.rs` | 监听 well-known name 的 `NameOwnerChanged`;无论名称变成无 owner,还是直接换给新 owner,只要注册时的 owner 失去该名称就注销旧 item |
| `src/client.rs` | 断开检测匹配 `NameOwnerChanged` 的 name (well-known/unique 都兼容),并同时处理无 owner 与 owner 直接换手;unregister 传目的名称 |
| `src/client.rs` | `handle_item` 出错时 `break` 改为 `continue`,单 item 失败不再杀死流 |

### 验证

electron 项正常出现:

```
Add("org.freedesktop.StatusNotifierItem-250182-1", StatusNotifierItem { id: "netease-cloud-music-web-player_status_icon_1", ..., title: "网易云音乐", ... })
```

菜单正常拉取 (MenuConnect + Menu),无 ERROR 日志。

完成 owner 生命周期处理后的自动化验证:

```
cargo fmt -- --check
cargo check --all-features
cargo test --all-features
```

上述命令均通过。现有测试主要覆盖编译和地址解析,owner 直接换手分支仍需要真实 D-Bus 集成测试或手工验证。

### 兼容性与生命周期说明

- well-known name 必须保留为 item 的对外地址,否则会重现上述 Electron/Chromium 的 `GetAll` 失败。
- unique owner 仍有用:它用于确认注册名称当前存在,并识别注册时的那个连接何时失去名称。它不应替换 item 对外公布的地址。
- D-Bus well-known name 可能从旧 owner 直接转移给新 owner,`NameOwnerChanged.new_owner` 此时不为空。因此不能只用 `new_owner.is_none()` 判断旧 item 已离开。
