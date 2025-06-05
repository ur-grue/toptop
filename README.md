# toptop

`toptop` is a terminal-based system monitor inspired by btop. It displays CPU, memory, disk, and network usage with a lightweight curses interface.

The enhanced version shows per-core CPU usage, current CPU frequency, memory consumption, disk read/write speeds, network transfer rates, and system uptime.

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

