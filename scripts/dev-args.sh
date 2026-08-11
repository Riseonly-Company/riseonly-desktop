#!/bin/sh
# Friendly names for the app's launch arguments, translated in one place.
#
# SOURCED, not executed: it rewrites the caller's positional parameters, so the
# launch line stays `"$@"` and every dev-* task gets the same vocabulary without
# repeating the table.
#
#   cargo make dev-macos --design     ->  Riseonly.app --args -riseStorybook
#
# The right-hand side is the app's own constant, LAUNCH_ARGUMENT in
# app/src/app/storybook/storybook.rs. A test there reads this file back, so
# renaming the constant fails the build instead of quietly opening the product.
#
# Anything unrecognised is passed through untouched, which is what keeps the raw
# arguments usable for the flags that have no friendly name yet.

for arg in "$@"; do
    shift
    case "$arg" in
        --design) set -- "$@" "-riseStorybook" ;;
        *) set -- "$@" "$arg" ;;
    esac
done
