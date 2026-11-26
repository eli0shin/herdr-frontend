"""An htop-style curses screen for record.py: colored meter bars redrawn in place, a boxed
table, and a status line, on the alternate screen. Deterministic: three frames, then it exits
on `q`."""

import curses


def frame(scr, step):
    loads = [(12 + step * 30) % 100, (55 + step * 20) % 100, (90 - step * 25) % 100]
    for i, load in enumerate(loads):
        bar = int(load * 0.4)
        scr.addstr(i, 0, f"{i + 1:>2}[")
        scr.addstr(i, 3, "|" * bar, curses.color_pair(1 if load < 70 else 2) | curses.A_BOLD)
        scr.addstr(i, 3 + bar, " " * (40 - bar))
        scr.addstr(i, 43, f"{load:>3}%]")
    scr.addstr(4, 0, f"Tasks: {40 + step}, load average: 0.{step}2", curses.color_pair(3))
    win = curses.newwin(6, 50, 6, 0)
    win.box()
    win.addstr(0, 2, " PID  NAME ", curses.A_REVERSE)
    for row, (pid, name) in enumerate([(101, "alpha"), (202, "beta"), (303, "gamma")]):
        win.addstr(row + 1, 2, f"{pid + step:>4}  {name:<10} {step * 3 + row}%")
    win.refresh()
    scr.addstr(curses.LINES - 1, 0, " F10 Quit ", curses.color_pair(4))
    scr.refresh()


def main(scr):
    curses.start_color()
    curses.init_pair(1, curses.COLOR_GREEN, curses.COLOR_BLACK)
    curses.init_pair(2, curses.COLOR_RED, curses.COLOR_BLACK)
    curses.use_default_colors()
    curses.init_pair(3, curses.COLOR_CYAN, -1)
    curses.init_pair(4, curses.COLOR_BLACK, curses.COLOR_CYAN)
    curses.curs_set(0)
    scr.timeout(300)
    step = 0
    while True:
        frame(scr, step)
        key = scr.getch()
        if key == ord("q"):
            break
        step = min(step + 1, 2)


curses.wrapper(main)
