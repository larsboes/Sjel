#!/bin/bash
set -euo pipefail
python3 "$(dirname "$0")/vault-archive-intake.test.py"
