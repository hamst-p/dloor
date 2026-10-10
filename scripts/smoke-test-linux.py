"""Check native Linux binaries without a desktop, real downloads, or user config."""

import errno
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

ANSI_SEQUENCE = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")


def attach_terminal():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def check_tui(binary, environment):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    process = subprocess.Popen(
        [str(binary)],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=environment,
        preexec_fn=attach_terminal,
    )
    os.close(slave)
    output = bytearray()
    sent_quit = False
    deadline = time.monotonic() + 20
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    chunk = os.read(master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
                if not chunk:
                    break
                output.extend(chunk)
                # Ratatui skips blank cells and uses cursor moves between words.
                visible = ANSI_SEQUENCE.sub(b"", output)
                if not sent_quit and b"InputURL" in b"".join(visible.split()):
                    os.write(master, b"q")
                    sent_quit = True
        try:
            result = process.wait(timeout=max(0.1, deadline - time.monotonic()))
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(
                f"TUI timed out (quit sent: {sent_quit}):\n"
                + ANSI_SEQUENCE.sub(b"", output).decode(errors="replace")
            ) from error
        if result != 0 or not sent_quit or b"\x1b[?1049l" not in output:
            raise RuntimeError(
                f"TUI did not render and exit cleanly (status {result}):\n"
                + output.decode(errors="replace")
            )
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        os.close(master)


def main():
    if sys.platform != "linux" or len(sys.argv) != 2:
        raise SystemExit("Usage on Linux: python3 scripts/smoke-test-linux.py <binary-dir>")
    binary_dir = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="dloor-smoke-") as directory:
        root = Path(directory)
        config = root / "config" / "dloor-tui"
        config.mkdir(parents=True)
        downloads = root / "downloads"
        downloads.mkdir()
        (config / "config.toml").write_text(
            'default_quality = "Best"\n'
            'clipboard_autofill = true\n'
            '[destination]\n'
            'type = "local"\n'
            f'path = "{downloads}"\n',
            encoding="utf-8",
        )
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        environment = os.environ.copy()
        environment.update(
            XDG_CONFIG_HOME=str(config.parent),
            XDG_RUNTIME_DIR=str(runtime),
            TERM="xterm-256color",
        )
        for name in ("DISPLAY", "WAYLAND_DISPLAY"):
            environment.pop(name, None)
        for option in ("--help", "--version"):
            result = subprocess.run(
                [str(binary_dir / "dloor-cli"), option],
                env=environment,
                capture_output=True,
                text=True,
                check=True,
                timeout=10,
            )
            if not result.stdout.strip() or result.stderr:
                raise RuntimeError(f"Unexpected CLI {option} output: {result}")
        check_tui(binary_dir / "dloor", environment)
    print("CLI help/version and headless TUI startup/quit passed")


if __name__ == "__main__":
    main()
