# winrec
轻量命令行录制工具
# 查看显示器列表
winrec --list

# 正斜杠 / 反斜杠都能正常识别，自动创建不存在的目录
winrec -o D:/captures/2026/demo1.mp4 -f 30
winrec -o D:\captures\2026\demo2.mp4 -f 60 -m 1

# 定时录 15 秒后自动停止保存
winrec -o test.mp4 -t 15
