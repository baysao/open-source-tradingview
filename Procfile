# TradeX dev stack, one line per service.
# Start it with: mise run dev        (db + hub; mise run dev:hub for one service)
#   or:         bash scripts/dev.sh dev [svc...]
#
# Every line goes through scripts/dev.sh, so build, env and ports stay in one place. Plain `bash`
# commands keep the file parseable by anything that reads a Procfile-shaped list; dev.sh puts its
# own bash first on PATH. mise.toml [tasks."dev:*"] wraps each line as a task.
db: bash scripts/dev.sh _svc db
hub: bash scripts/dev.sh _svc hub
# Production only: idles with a message where cloudflared is not installed.
tunnel: bash scripts/dev.sh _svc tunnel
