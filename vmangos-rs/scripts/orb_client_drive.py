#!/usr/bin/env python3
"""Drive the stock 1.12.1 client on the OrbStack wow-rig-arm VM.

Rebuilt rig tooling (the original /tmp drive scripts were lost):
launches WoW.exe under Box64 + Wine 10 on Xvfb :99, drives it with
xdotool, and captures screenshots to the Mac /tmp via /mnt/mac.

Usage: orb_client_drive.py <step> [args]
Steps are composable shell-outs used interactively by the agent:
  shot <name>            screenshot to /tmp/<name>.png
  click <x> <y>          left click
  rclick <x> <y>         right click
  type <text>            type text
  key <keysym...>        e.g. key Return, key Tab
  hold <key> <secs>      hold a movement key
  login                  TESTUSER login from the login screen
  status                 client process + window list
"""
import subprocess, sys, time

MACHINE = "wow-rig-arm"

def vm(cmd: str) -> str:
    out = subprocess.run(
        ["orbctl", "run", "-m", MACHINE, "sh", "-lc", cmd],
        capture_output=True, text=True, check=False,
    )
    return (out.stdout + out.stderr).strip()

def shot(name: str) -> None:
    vm(f"DISPLAY=:99 import -window root /mnt/mac/tmp/{name}.png")
    print(f"/tmp/{name}.png")

def click(x: int, y: int, button: int = 1) -> None:
    vm(f"DISPLAY=:99 xdotool mousemove {x} {y} click {button}")
    time.sleep(0.6)

def typetext(text: str) -> None:
    vm(f"DISPLAY=:99 xdotool type --delay 60 -- {shellquote(text)}")
    time.sleep(0.3)

def key(*keys: str) -> None:
    for k in keys:
        vm(f"DISPLAY=:99 xdotool key {k}")
        time.sleep(0.25)

def hold(k: str, secs: float) -> None:
    vm(f"DISPLAY=:99 xdotool keydown {k}")
    time.sleep(secs)
    vm(f"DISPLAY=:99 xdotool keyup {k}")

def shellquote(s: str) -> str:
    return "'" + s.replace("'", "'\"'\"'") + "'"

def login() -> None:
    click(512, 413)          # account field
    vm("DISPLAY=:99 xdotool key ctrl+a")
    typetext("TESTUSER")
    key("Tab")
    typetext("password123")
    key("Return")
    time.sleep(6)
    shot("gnome-01-charselect")

def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    step, args = sys.argv[1], sys.argv[2:]
    if step == "shot":
        shot(args[0])
    elif step == "click":
        click(int(args[0]), int(args[1]))
    elif step == "rclick":
        click(int(args[0]), int(args[1]), button=3)
    elif step == "type":
        typetext(args[0])
    elif step == "key":
        key(*args)
    elif step == "hold":
        hold(args[0], float(args[1]))
    elif step == "login":
        login()
    elif step == "status":
        print(vm("ps aux | grep -i WoW.exe | grep -v grep | head -3; DISPLAY=:99 xdotool search --onlyvisible --name . getwindowname %@ 2>/dev/null | head"))
    else:
        print(f"unknown step {step}")
        return 2
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
