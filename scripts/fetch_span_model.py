# Copyright (C) 2026 Toit contributors.
"""Download the pinned Apache-2.0 T5-base checkpoint for offline span proposals."""
from fetch_context_model import main

REPOSITORY = 'google-t5/t5-base'
REVISION = 'a9723ea7f1b39c1eae772870f3b547bf6ef7e6c1'
EXPECTED = {
    'README.md': 'c43c23f205b6839a6adb10f0757265f76c1c6f96b7793a4e53582d3a61cbfb23',
    'config.json': '46dd7cb62d29c81fb551e0ef1ea274c24a46ba441eeb948897706252933df033',
    'spiece.model': 'd60acb128cf7b7f2536e8f38a5b18a05535c9e14c7a355904270e15b0945ea86',
    'tokenizer.json': 'd2acde0d8d71dd30a711834b07781b9c89feaac33fd332f60507699282740066',
    'model.safetensors': 'a90903540cc02cbeb7ff9f823f1a80eb778c7e22426a0e620b01c77a5ec8f5b4',
}

if __name__ == '__main__':
    main(REPOSITORY, REVISION, EXPECTED)
