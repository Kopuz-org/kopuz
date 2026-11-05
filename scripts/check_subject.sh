#!/usr/bin/env bash
# Usage: check_subject.sh <what> <subject>. Fails, with a GitHub annotation, unless the subject is scoped.
set -euo pipefail

what=$1
subject=$2
conventional_re='^(feat|fix|refactor|perf|test|style|ci|revert)(\([^)]+\)!?|!): '
scoped_re='^[a-z0-9][a-z0-9._-]*(/[a-z0-9][a-z0-9._-]*)?: .+'

if ((${#subject} > 80)); then
  echo "::error title=Subject too long::$what has ${#subject} characters: $subject"
  exit 1
fi

if [[ "$subject" =~ $conventional_re ]]; then
  echo "::error title=Conventional Commit subject::$what looks like Conventional Commits; Kopuz uses scoped commits: $subject"
  exit 1
fi

if ! [[ "$subject" =~ $scoped_re ]]; then
  echo "::error title=Invalid subject::$what must use 'scope: description': $subject"
  exit 1
fi
