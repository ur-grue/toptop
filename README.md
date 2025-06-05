# toptop

`toptop` is a terminal-based system monitor inspired by btop. It displays CPU, memory, disk, and network usage with a lightweight curses interface.

The latest version adds a colourful gruvbox theme. CPU usage is shown with per-core bars, while disk and network throughput are plotted using small graphs so you can see trends over time. It still shows CPU frequency, memory consumption, and system uptime.

## Installation

Create a virtual environment and install dependencies:

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
```

## Usage

Run the monitor with:

```bash
python -m toptop.main
```

Press `q` to quit.

