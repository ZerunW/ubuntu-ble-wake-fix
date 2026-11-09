# ubuntu-ble-wake-fix

[English](README.md) | 简体中文

修复 Ubuntu 上低功耗蓝牙（BLE）键盘和鼠标无法把电脑从休眠中唤醒的问题。装上以后，按一下键盘就能唤醒电脑。

在 Ubuntu 24.04 + 较新的内核（HWE 7.0）上，低功耗蓝牙（BLE）键盘没法把电脑从休眠中唤醒，哪怕
`bluetoothctl info` 显示 `WakeAllowed: yes`（或者根本没有 `WakeAllowed` 这一项）。内核用每个设备的
一个标记（Remote Wakeup，远程唤醒）决定能不能唤醒，而 BlueZ 5.72 要么从不设置它，要么刚设完又清掉。
`ubuntu-ble-wake-fix` 是一个以 root 运行的小服务：替已配对的输入设备保持这个标记，并打开蓝牙控制器的 USB 唤醒。

- **不修改 BlueZ。** 它直接使用内核的蓝牙管理接口（bluetoothd 用的也是这个稳定接口）。BlueZ 更新不影响
  它；以后 BlueZ 自己设对了，它就什么都不用做。
- **只管输入设备。** 已绑定、并且提供 HID 服务（HID over GATT 或经典 HID）或外观是键盘/鼠标的设备。
  耳机之类不受影响，也可以单独排除某个设备。
- **即时生效。** 内核会通知每一次标记变化；如果有程序清掉了远程唤醒，几毫秒内就会被补回。每分钟还会全面
  检查一次。

## 安装

从 [最新发布](https://github.com/ZerunW/ubuntu-ble-wake-fix/releases/latest) 下载 `.deb`，然后：

```bash
sudo apt install ./ubuntu-ble-wake-fix_*_amd64.deb
```

服务会立刻启动，之后每次开机自动运行（`systemctl status ubuntu-ble-wake-fix`）。

查看它看到的情况（不做任何修改）：

```bash
sudo ubuntu-ble-wake-fix --check
```

然后休眠电脑，稍等片刻按一下键盘。

## 它会改什么，代价是什么

- 每个已配对的蓝牙键盘、鼠标（以及其他输入设备）都会被打开内核的 Remote Wakeup（远程唤醒）标记。
  服务会一直保持它打开：别的程序把它关掉，服务会再打开。不想让某个设备唤醒电脑，就把它写进下面的排除列表。
- 蓝牙控制器和它上游的 USB 集线器会被打开 USB 唤醒（`--no-usb` 可以关掉，但那样蓝牙就唤醒不了电脑了）。
- 所以碰一下鼠标、或者放在包里的键盘被压到，也可能把电脑唤醒；允许唤醒的蓝牙控制器在休眠时也可能多耗一点电。
  用不到的设备请排除。

## 配置

`/etc/ubuntu-ble-wake-fix/exclude`：不允许唤醒电脑的设备地址，一行一个（`#` 后面是注释）。改完后运行
`sudo systemctl restart ubuntu-ble-wake-fix`。已经设置的标记会保留到 bluetoothd 或蓝牙控制器重启。

## 卸载

```bash
sudo apt remove ubuntu-ble-wake-fix   # 或者 apt purge，会连排除列表一起删除
```

卸载会停止服务，但不会撤销它已经设置的东西：远程唤醒标记会保留到 bluetoothd 或蓝牙控制器重启
（`sudo systemctl restart bluetooth`），USB 唤醒会保留到下次重启电脑。

## 分享日志

`journalctl -u ubuntu-ble-wake-fix` 和 `--check` 的输出里有你设备的蓝牙地址和名称。
在 issue 里贴出来之前，请把地址换掉（比如换成 `C0:FF:EE:00:00:01`）。

## 原理

详见 [docs/how-it-works.md](docs/how-it-works.md)（英文，包括 btmon 记录和 BlueZ 的两个 bug）。

## 许可证

GPL-2.0-or-later，见 [LICENSE](LICENSE)。
