# Tasks

## 1. Storage
- [x] 1.1 SQLite database with `vms` and `settings` tables, WAL and `user_version`
- [x] 1.2 Import `config.json` and older VM files once, renaming them to `.migrated`
- [x] 1.3 Read-only `vm.json` snapshot in each VM's folder

## 2. Workspace and folders
- [x] 2.1 App-wide Workspace preference (default `~/macOS-VMs`) and `<Workspace>/<name>` folders
- [x] 2.2 Move a VM's own files into a new folder, with rollback on error
- [x] 2.3 First-run VM created inside the Workspace

## 3. GUI
- [x] 3.1 New VM wizard (name and version, hardware, storage, summary)
- [x] 3.2 Edit, Delete with optional purge, Move to Workspace
- [x] 3.3 Export/Import of `.tar` bundles and `.json` settings with progress
- [x] 3.4 Theme selector applied at once, toolbar recolored on change

## 4. CLI and safety
- [x] 4.1 `vm list|status|create|select|move|export|import|delete`, `workspace show|set`, `--vm`
- [x] 4.2 Running detection through `/proc`; Start refuses a disk in use
- [x] 4.3 Bundle import accepts only known file names (path traversal test)

## 5. Docs
- [x] 5.1 README, getting started, CLI, architecture and screenshots
