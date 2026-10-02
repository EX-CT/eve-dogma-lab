#!/bin/bash
# local Pyfa oracle (GPL test tool, not committed): run.sh case.json ... > out.jsonl
PYFA=/workspace/exct-eve/ref/pyfa PYTHONPATH=/workspace/exct-eve/ref/stubs /workspace/exct-eve/ref/pyfa-venv/bin/python /workspace/exct-eve/lab-h/oracle_local/${ORACLE:-pyfa_oracle.py} "$@"
