### Windows specifics: paths too long for Windows, git's long paths, git inside WSL.

windows-long-paths = Paths too long for Windows
windows-long-paths-description = Which tracked files are beyond the 260 characters Windows takes without long paths, at the place the worktree is: how many, the longest and its length; whether git's core.longpaths is on, whether the limit holds on this system (applies) and so whether git is kept from them (blocks_git)
windows-long-paths-enable = Turn on long paths
windows-long-paths-enable-description = Set git's core.longpaths, so that Git for Windows takes paths longer than 260 characters; params: scope (repo|user, repo when absent: whose configuration is written; the user's holds for every clone made afterwards too)
windows-wsl = Git inside WSL
windows-wsl-description = Whether the repository is inside a WSL distribution (a \\wsl.localhost\ or \\wsl$\ path), and if so the distribution, the Linux path and the git that is run for it there
