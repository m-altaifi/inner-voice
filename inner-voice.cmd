@echo off
rem Double-click this, or pin it. Everything comes from .env beside it; any
rem extra flags pass straight through, e.g. inner-voice.cmd --setup
"%~dp0target\release\inner-voice.exe" %*
