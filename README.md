# vdisplay

在 Sunshine 中通过应用的 **前/后置命令（prep-cmd）** 实现 Apollo 式的虚拟显示器：串流开始时按客户端分辨率/帧率/HDR 创建虚拟显示器，结束时移除。

底层调用的是 Apollo 使用的同一个驱动 —— [SudoVDA](https://github.com/SudoMaker/SudoVDA)（`SudoMaker Virtual Display Adapter`）。需要先安装该驱动（装过 Apollo 即已安装）。

## 常用命令（Sunshine 前/后置命令）

假设 `vdisplay.exe` 放在 `C:\Tools\`。以下命令会创建一个与客户端（Moonlight）请求**一致的分辨率、帧率、HDR 状态**的虚拟显示器，并设为主显示器供 Sunshine 采集：

```
do:   cmd /C "C:\Tools\vdisplay.exe start --width %SUNSHINE_CLIENT_WIDTH% --height %SUNSHINE_CLIENT_HEIGHT% --fps %SUNSHINE_CLIENT_FPS% --hdr %SUNSHINE_CLIENT_HDR% --name "%SUNSHINE_CLIENT_NAME%" --primary"
undo: C:\Tools\vdisplay.exe stop
```

| 参数 | 来源（Sunshine 注入的环境变量） |
|---|---|
| `--width` / `--height` | `SUNSHINE_CLIENT_WIDTH` / `SUNSHINE_CLIENT_HEIGHT`：客户端请求的分辨率 |
| `--fps` | `SUNSHINE_CLIENT_FPS`：客户端请求的帧率 |
| `--hdr` | `SUNSHINE_CLIENT_HDR`：`true`/`false`，客户端是否开启 HDR |
| `--name` | `SUNSHINE_CLIENT_NAME`：客户端名称，用作显示器名称并派生显示器 GUID（每个客户端单独记忆缩放等设置） |
| `--primary` | 设为主显示器；换成 `--exclusive` 则串流期间关闭物理屏 |

`vdisplay` 不带参数时也会自己读取这些环境变量，所以下面的简写效果完全相同，还不会经过 `cmd`：

```
do:   C:\Tools\vdisplay.exe start --primary
undo: C:\Tools\vdisplay.exe stop
```

对应的 `sunshine.conf` 全局写法：

```
global_prep_cmd = [{"do":"C:\\Tools\\vdisplay.exe start --primary","undo":"C:\\Tools\\vdisplay.exe stop","elevated":false}]
```

同时请把 `output_name` 留空、`dd_configuration_option` 设为禁用（详见下文"Sunshine 配置"）。

## 工作原理

SudoVDA 有 watchdog：默认 3 秒内没有收到任何 IOCTL 就会移除**所有**虚拟显示器。因此：

- `vdisplay start` 拉起一个脱离的后台 daemon，由它创建显示器、设置模式/主屏/HDR，并持续 ping 驱动；显示器就绪后 `start` 退出，Sunshine 继续启动串流。
- `vdisplay stop` 通过命名事件通知 daemon 移除显示器并恢复原来的显示布局。
- 若 Sunshine 进程退出（或 daemon 被杀掉），显示器也会被自动移除。
- 显示器移除后 Windows 会从自身的显示配置数据库恢复原布局；即便 `--exclusive` 关掉了物理屏，daemon 异常退出时也能自愈。

注意：watchdog 是驱动全局的，**任何**客户端的 ping 都会续命。如果同时运行着 Apollo，daemon 被强杀后虚拟屏不会被 watchdog 移除；此时执行一次 `vdisplay stop`（或下一次 `start`）会根据 `vdisplay.state` 里记录的 GUID 清理残留显示器。

`scripts/test-exclusive.ps1` 可以验证 `--exclusive` 的关闭与恢复（恢复失败时会兜底执行 `DisplaySwitch.exe /extend`）。

显示器 GUID 由客户端名称（`SUNSHINE_CLIENT_NAME`）派生，因此 Windows 会按客户端记住缩放等设置（Apollo 用的是客户端 UUID，Sunshine 不提供）。

## 构建

```sh
cargo build --release
# 产物：target/release/vdisplay.exe
```

## Sunshine 配置

1. 把 `vdisplay.exe` 放在一个当前用户可写的目录（日志 `vdisplay.log` 写在 exe 旁边，不可写时写到 `%TEMP%`）。
2. Sunshine 设置：
   - **Audio/Video → 显示器设备 ID（`output_name`）留空**：Sunshine 会采集主显示器，配合 `--primary` 即采集虚拟屏。
   - **显示器设备配置（`dd_configuration_option`）设为禁用**：Sunshine 的显示器配置在 prep 命令之前执行，此时虚拟屏还不存在。
3. 在应用（或全局）的前置命令中添加：

   | 字段 | 值 |
   |---|---|
   | do   | `C:\Tools\vdisplay.exe start --primary` |
   | undo | `C:\Tools\vdisplay.exe stop` |

   do/undo 的"以管理员身份运行"要保持一致。

## 命令

```
vdisplay start [options]   创建虚拟显示器（Sunshine do 命令）
vdisplay stop              移除虚拟显示器（Sunshine undo 命令）
vdisplay status            查看状态与日志路径

start 选项（默认值来自 Sunshine 的 SUNSHINE_CLIENT_* 环境变量）：
  --width <px>          宽度              [SUNSHINE_CLIENT_WIDTH，否则 1920]
  --height <px>         高度              [SUNSHINE_CLIENT_HEIGHT，否则 1080]
  --fps <hz>            刷新率，如 60 / 59.94 [SUNSHINE_CLIENT_FPS，否则 60]
  --hdr [true|false]    强制开/关 HDR（也可用 --no-hdr） [SUNSHINE_CLIENT_HDR，否则不修改]
  --primary             设为主显示器
  --exclusive           串流期间关闭其他所有显示器（隐含 --primary）
  --name <text>         显示器名称/身份   [SUNSHINE_CLIENT_NAME]
  --id <text>           用于派生显示器 GUID 的稳定标识 [--name]
  --adapter <gpu>       指定渲染 GPU，如 "NVIDIA GeForce RTX 4090"
  --no-watch            Sunshine 退出时不自动移除
```
