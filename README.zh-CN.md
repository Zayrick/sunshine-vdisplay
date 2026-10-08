# vdisplay

[English](README.md) | **简体中文**

为 [Sunshine](https://github.com/LizardByte/Sunshine) 提供 Apollo 式的虚拟显示器。

`vdisplay` 是一个 Windows 命令行小工具，作为 Sunshine 的 **前/后置命令（prep-cmd）** 使用：串流开始时按客户端的分辨率、帧率和 HDR 状态创建虚拟显示器，串流结束时移除 —— 效果与 [Apollo](https://github.com/ClassicOldSong/Apollo) 相同，但使用的是原版 Sunshine。

底层使用与 Apollo 相同的驱动：[SudoVDA](https://github.com/SudoMaker/SudoVDA)（SudoMaker Virtual Display Adapter），驱动已内置在 `vdisplay.exe` 中。

## 功能

- 按需创建虚拟显示器，自动匹配客户端的**分辨率、帧率和 HDR** 设置
- 可设为**主显示器**，或作为串流期间**唯一**启用的显示器（`--exclusive`）
- 按客户端区分显示器身份，Windows 会为每个客户端分别记住缩放等设置
- 串流结束、Sunshine 退出或 `vdisplay` 异常退出时自动清理
- 内置 SudoVDA 驱动，一条命令即可安装/卸载
- 单个独立 `.exe`，串流时无需管理员权限

## 系统要求

- Windows 10/11 x64
- Sunshine
- SudoVDA 驱动 —— 装过 Apollo 的话驱动已存在

## 快速上手

1. 从 [Releases](../../releases/latest) 页面**下载** `vdisplay.exe`，放到当前用户可写的目录，例如 `C:\Tools\`（日志 `vdisplay.log` 写在 exe 旁边，不可写时写到 `%TEMP%`）。

2. **安装驱动**（只需一次，会弹出 UAC）：

   ```
   C:\Tools\vdisplay.exe install-driver
   ```

3. **配置 Sunshine**（Web UI → 配置）：
   - *音频/视频 → 显示器设备 ID*（`output_name`）：**留空**，让 Sunshine 采集主显示器。
   - *音频/视频 → 设备配置*（`dd_configuration_option`）：**禁用**。Sunshine 会在前置命令之前应用该配置，那时虚拟显示器还不存在。
   - *常规 → 命令准备工作*，添加（全局或单个应用均可）：

     | 字段 | 值 |
     |---|---|
     | 执行命令（do） | `C:\Tools\vdisplay.exe start --primary` |
     | 撤销命令（undo） | `C:\Tools\vdisplay.exe stop` |

     两条命令的"以管理员身份运行"要保持一致。

   或在 `sunshine.conf` 中这样写：

   ```
   global_prep_cmd = [{"do":"C:\\Tools\\vdisplay.exe start --primary","undo":"C:\\Tools\\vdisplay.exe stop","elevated":false}]
   ```

4. 用 Moonlight 连接。此时会出现一个与客户端分辨率/帧率/HDR 一致的虚拟显示器并成为主显示器；结束串流后自动移除。

把 `--primary` 换成 `--exclusive` 可以在串流期间关闭所有物理显示器，结束后自动恢复。

## 命令

```
vdisplay start [选项]      创建虚拟显示器（Sunshine 执行命令）
vdisplay stop              移除虚拟显示器（Sunshine 撤销命令）
vdisplay status            查看版本、是否有虚拟显示器在运行、驱动状态与日志路径
vdisplay install-driver    （重新）安装内置的 SudoVDA 驱动（会请求管理员权限）
vdisplay uninstall-driver  卸载 SudoVDA：移除设备、驱动包与证书（会请求管理员权限）
vdisplay version           输出版本号
```

### `start` 选项

默认值取自 Sunshine 为前置命令注入的 `SUNSHINE_CLIENT_*` 环境变量，所以通常只写 `start --primary` 就够了。

| 选项 | 说明 | 默认值 |
|---|---|---|
| `--width <px>` | 宽度 | `SUNSHINE_CLIENT_WIDTH`，否则 1920 |
| `--height <px>` | 高度 | `SUNSHINE_CLIENT_HEIGHT`，否则 1080 |
| `--fps <hz>` | 刷新率，如 `60` 或 `59.94` | `SUNSHINE_CLIENT_FPS`，否则 60 |
| `--hdr [true\|false]`、`--no-hdr` | 强制开/关 HDR | `SUNSHINE_CLIENT_HDR`，否则不修改 |
| `--primary` | 设为主显示器 | 关 |
| `--exclusive` | 串流期间关闭其他所有显示器（隐含 `--primary`） | 关 |
| `--name <text>` | 显示器名称/身份 | `SUNSHINE_CLIENT_NAME` |
| `--id <text>` | 用于派生显示器 GUID 的稳定标识 | `--name` |
| `--adapter <gpu>` | 按名称指定渲染 GPU，如 `"NVIDIA GeForce RTX 4090"` | 系统默认 |
| `--no-watch` | `sunshine.exe` 退出时不自动移除显示器 | 关 |

显式传参的写法效果相同：

```
cmd /C "C:\Tools\vdisplay.exe start --width %SUNSHINE_CLIENT_WIDTH% --height %SUNSHINE_CLIENT_HEIGHT% --fps %SUNSHINE_CLIENT_FPS% --hdr %SUNSHINE_CLIENT_HDR% --name "%SUNSHINE_CLIENT_NAME%" --primary"
```

## 工作原理

SudoVDA 带有 watchdog：超时时间内（默认 3 秒）没有收到任何请求，就会移除**所有**虚拟显示器。因此：

- `vdisplay start` 拉起一个脱离的后台 daemon，由它创建显示器、设置模式/主屏/HDR，并持续 ping 驱动。显示器就绪后 `start` 立即返回，Sunshine 继续启动串流。
- `vdisplay stop` 通过命名事件通知 daemon 移除显示器并恢复原来的显示布局。
- 若 Sunshine 退出或 daemon 被杀掉，ping 停止，驱动会移除显示器。随后 Windows 从自身的显示配置数据库恢复原布局 —— 被 `--exclusive` 关掉的物理屏也会恢复。

watchdog 是驱动全局的，**任何**客户端的 ping 都会让显示器保持存活。如果同时运行着 Apollo，daemon 被强杀后残留的显示器不会被 watchdog 移除；下一次 `vdisplay stop` 或 `start` 会根据 `vdisplay.state` 中记录的 GUID 清理它。

显示器 GUID 由客户端名称派生，因此 Windows 会按客户端记住缩放等设置。（Apollo 使用客户端 UUID，但 Sunshine 不提供该信息。）

## 驱动

`drivers/sudovda/` 下是 Apollo 打包的 SudoVDA 驱动（`SudoVDA.inf/.dll/.cat/.cer`），编译时嵌入 exe。`install-driver` 的流程与 Apollo 的 `install.bat` 相同，只是直接调用 SetupAPI，不再依赖 `nefconc.exe`：

1. 将 `sudovda.cer` 导入本机 `Root` 和 `TrustedPublisher` 证书存储；
2. 若不存在则创建 `Root\SudoMaker\SudoVDA` 设备节点（Display 类）；
3. 用 `UpdateDriverForPlugAndPlayDevices` 在该节点上安装 `SudoVDA.inf`。

`start` 不会安装驱动，检测不到驱动时直接报错退出，因此串流本身永远不需要管理员权限。

`uninstall-driver` 依次执行：停止正在运行的虚拟显示器 → 移除设备节点 → 从驱动存储中删除驱动包（`oemNN.inf`）→ 移除证书。**Apollo 与本工具使用同一个驱动**，卸载后 Apollo 的虚拟显示器也将无法使用。

## 故障排查

- 运行 `vdisplay status` 查看版本、驱动状态和日志文件位置。
- 每次运行都会写入 `vdisplay.log`，反馈问题时请附上该日志。
- `scripts/test-exclusive.ps1` 可验证 `--exclusive` 能否正确关闭并恢复物理显示器（恢复失败时会兜底执行 `DisplaySwitch.exe /extend`）。

## 构建

需要 Rust 工具链（edition 2024，Rust 1.85+）及 MSVC 目标。

```sh
cargo build --release
# 产物：target/release/vdisplay.exe
```

`vdisplay version` 显示的版本号在编译时取自环境变量 `VDISPLAY_VERSION`，未设置时使用 `Cargo.toml` 中的版本：

```sh
VDISPLAY_VERSION=1.2.0 cargo build --release
```

### 发布

发布由 GitHub Actions 完成（[`.github/workflows/release.yml`](.github/workflows/release.yml)），两种方式任选其一：

- 推送形如 `v1.2.0` 的 tag；或
- 在 Actions 页面手动运行 **Release** workflow 并填写版本号（如 `1.2.0`），会在所选分支上创建 tag `v1.2.0`。

workflow 会编译嵌入该版本号的 `vdisplay.exe`，并将 `vdisplay-v<版本>-windows-x64.zip` 及其 SHA-256 校验文件发布到 GitHub Release。带后缀的版本（如 `1.2.0-beta.1`）会标记为预发布。

## 许可证

[GPL-3.0](LICENSE)。

内置的 SudoVDA 驱动由 [SudoMaker](https://github.com/SudoMaker/SudoVDA) 开发，按 [Apollo](https://github.com/ClassicOldSong/Apollo) 中附带的版本重新分发。
