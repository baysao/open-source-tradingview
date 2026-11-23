# TradeX dev stack, run by gaffa (https://github.com/oryon-dominik/interstellar-gaffa).
# Start it with: bash scripts/dev.sh dev [svc...]
#
# Every line goes through scripts/dev.sh, so build, env and ports stay in one place. Lines are
# plain `bash ...` commands so they parse the same in gaffa's default shell everywhere (pwsh on
# Windows, sh on Unix); dev.sh puts its own bash first on PATH. Do not pass gaffa --shell bash on
# Windows: gaffa 2026.11.1 then drops the arguments after the script name.
hub: bash scripts/dev.sh _svc hub
