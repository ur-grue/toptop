import asyncio
import time
import psutil
import curses
from dataclasses import dataclass

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
    curses.start_color()
    curses.use_default_colors()
    curses.init_pair(1, curses.COLOR_CYAN, -1)

    prev_disk = psutil.disk_io_counters()
    prev_net = psutil.net_io_counters()
    prev_time = time.time()

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

        stdscr.erase()
        stdscr.addstr(0, 0, "toptop - enhanced monitor", curses.color_pair(1))
        stdscr.addstr(2, 0, f"Uptime: {stats.uptime}")
        stdscr.addstr(4, 0, f"CPU Frequency: {stats.cpu_freq:.0f} MHz")
        for idx, pct in enumerate(stats.cpu_percents):
            stdscr.addstr(5 + idx, 0, f"CPU{idx}: {pct:5.1f}%")
        mem_line = 6 + len(stats.cpu_percents)
        stdscr.addstr(mem_line, 0, f"Memory: {stats.mem_used/1e6:.0f}M / {stats.mem_total/1e6:.0f}M")
        stdscr.addstr(mem_line + 1, 0, f"Disk Usage: {stats.disk_percent:.1f}%")
        stdscr.addstr(mem_line + 2, 0, f"Disk R/W: {disk_read/1e6:.1f}M/s {disk_write/1e6:.1f}M/s")
        stdscr.addstr(mem_line + 3, 0, f"Net Up/Down: {net_sent/1e6:.1f}M/s {net_recv/1e6:.1f}M/s")
        stdscr.addstr(mem_line + 5, 0, "Press 'q' to quit")

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
