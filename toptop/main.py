import asyncio
import time
import psutil
import curses
from dataclasses import dataclass

BLOCKS = " ▁▂▃▄▅▆▇█"

def usage_bar(pct: float, width: int = 20) -> str:
    filled = int(pct / 100 * width)
    return "█" * filled + " " * (width - filled)

def sparkline(history: list[float], max_value: float, width: int) -> str:
    if max_value <= 0:
        max_value = 1
    result = []
    for val in history[-width:]:
        level = int(val / max_value * (len(BLOCKS) - 1))
        result.append(BLOCKS[level])
    if len(result) < width:
        result = [" "] * (width - len(result)) + result
    return "".join(result)

def setup_colors():
    curses.start_color()
    curses.use_default_colors()
    if curses.COLORS >= 256:
        curses.init_pair(1, 208, -1)  # orange
        curses.init_pair(2, 142, -1)  # green
        curses.init_pair(3, 109, -1)  # blue
        curses.init_pair(4, 175, -1)  # purple
        curses.init_pair(5, 223, -1)  # light text
    else:
        curses.init_pair(1, curses.COLOR_YELLOW, -1)
        curses.init_pair(2, curses.COLOR_GREEN, -1)
        curses.init_pair(3, curses.COLOR_BLUE, -1)
        curses.init_pair(4, curses.COLOR_MAGENTA, -1)
        curses.init_pair(5, curses.COLOR_WHITE, -1)

@dataclass
class SystemStats:
    cpu_percents: list[float]
    cpu_freq: float
    mem_used: int
    mem_total: int
    disk_percent: float
    uptime: str

def gather_stats() -> SystemStats:
    cpu_percents = psutil.cpu_percent(interval=None, percpu=True)
    freq = psutil.cpu_freq()
    mem = psutil.virtual_memory()
    disk = psutil.disk_usage('/')
    uptime_seconds = time.time() - psutil.boot_time()
    uptime = time.strftime('%H:%M:%S', time.gmtime(uptime_seconds))
    cpu_freq = freq.current if freq else 0.0
    return SystemStats(
        cpu_percents=cpu_percents,
        cpu_freq=cpu_freq,
        mem_used=mem.used,
        mem_total=mem.total,
        disk_percent=disk.percent,
        uptime=uptime,
    )

async def refresh_screen(stdscr):
    curses.curs_set(0)
    stdscr.nodelay(True)
    setup_colors()

    prev_disk = psutil.disk_io_counters()
    prev_net = psutil.net_io_counters()
    prev_time = time.time()
    net_up_hist: list[float] = []
    net_down_hist: list[float] = []
    disk_read_hist: list[float] = []
    disk_write_hist: list[float] = []
    graph_width = 30

    while True:
        now_disk = psutil.disk_io_counters()
        now_net = psutil.net_io_counters()
        now_time = time.time()
        interval = max(now_time - prev_time, 0.001)
        disk_read = (now_disk.read_bytes - prev_disk.read_bytes) / interval
        disk_write = (now_disk.write_bytes - prev_disk.write_bytes) / interval
        net_sent = (now_net.bytes_sent - prev_net.bytes_sent) / interval
        net_recv = (now_net.bytes_recv - prev_net.bytes_recv) / interval
        prev_disk, prev_net, prev_time = now_disk, now_net, now_time

        stats = gather_stats()

        disk_read_hist.append(disk_read)
        disk_write_hist.append(disk_write)
        net_up_hist.append(net_sent)
        net_down_hist.append(net_recv)
        for hist in (disk_read_hist, disk_write_hist, net_up_hist, net_down_hist):
            if len(hist) > graph_width:
                hist.pop(0)

        disk_max = max(max(disk_read_hist, default=0), max(disk_write_hist, default=0))
        net_max = max(max(net_up_hist, default=0), max(net_down_hist, default=0))

        stdscr.erase()
        stdscr.addstr(0, 0, "toptop - gruvbox monitor", curses.color_pair(1) | curses.A_BOLD)
        stdscr.addstr(2, 0, f"Uptime: {stats.uptime}", curses.color_pair(5))
        stdscr.addstr(4, 0, f"CPU Frequency: {stats.cpu_freq:.0f} MHz", curses.color_pair(5))
        line = 5
        for idx, pct in enumerate(stats.cpu_percents):
            bar = usage_bar(pct)
            stdscr.addstr(line, 0, f"CPU{idx}: ", curses.color_pair(5))
            stdscr.addstr(bar, curses.color_pair(2))
            stdscr.addstr(f" {pct:5.1f}%", curses.color_pair(5))
            line += 1
        mem_line = line
        stdscr.addstr(mem_line, 0, f"Memory: {stats.mem_used/1e6:.0f}M / {stats.mem_total/1e6:.0f}M", curses.color_pair(5))
        stdscr.addstr(mem_line + 1, 0, f"Disk Usage: {stats.disk_percent:.1f}%", curses.color_pair(5))
        stdscr.addstr(mem_line + 2, 0, f"Disk R/s: {disk_read/1e6:.1f}M/s  Disk W/s: {disk_write/1e6:.1f}M/s", curses.color_pair(4))
        stdscr.addstr(mem_line + 3, 0, sparkline(disk_read_hist, disk_max, graph_width), curses.color_pair(4))
        stdscr.addstr(mem_line + 4, 0, sparkline(disk_write_hist, disk_max, graph_width), curses.color_pair(4))
        stdscr.addstr(mem_line + 6, 0, f"Net Up: {net_sent/1e6:.1f}M/s  Net Down: {net_recv/1e6:.1f}M/s", curses.color_pair(3))
        stdscr.addstr(mem_line + 7, 0, sparkline(net_up_hist, net_max, graph_width), curses.color_pair(3))
        stdscr.addstr(mem_line + 8, 0, sparkline(net_down_hist, net_max, graph_width), curses.color_pair(3))
        stdscr.addstr(mem_line + 10, 0, "Press 'q' to quit", curses.color_pair(5))

        stdscr.refresh()
        try:
            ch = stdscr.getch()
            if ch == ord('q'):
                break
        except curses.error:
            pass
        await asyncio.sleep(1)


def main():
    curses.wrapper(lambda stdscr: asyncio.run(refresh_screen(stdscr)))

if __name__ == "__main__":
    main()
