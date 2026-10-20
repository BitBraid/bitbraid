### Patches.

## The commands: what the palette and the command line call each, and what it does.

patch-export = Create patch
patch-export-description = Write patches out; params: one of commits (each by itself), from (with to, HEAD when absent: the commits between), staged (bool) or worktree (bool: what is changed and not staged); one of dir (commits only, one file each) or file (a mailbox, or the diff); overwrite (bool: replace files that exist)
patch-check = Check patch
patch-check-description = Whether a patch file would apply, and the files it touches with lines added and deleted; changes nothing; params: file, index (bool), three_way (bool), reverse (bool)
patch-apply = Apply patch
patch-apply-description = Apply a patch file to the files; params: file, index (bool: to the index too), three_way (bool: merge where it does not apply as it is, which may leave conflicts), reverse (bool: take the change out)
patch-mailbox = Apply mailbox
patch-mailbox-description = Apply the patches of a mailbox file as commits on the current branch; stops in progress at a patch that conflicts or does not apply; params: file, three_way (bool: merge where a patch does not apply as it is; as git is configured when absent), no_verify (leave out the applypatch-msg and pre-applypatch hooks; the ones that were there are named in skipped_hooks)

## Journal entries: what the undo menu and the activity log call each change.
## A name is the file's last part, without the folders it is in.

patch-entry-apply = Apply patch { $name }
patch-entry-apply-reverse = Apply patch { $name } in reverse
patch-entry-mailbox = Apply mailbox { $name }

## Refusals: what follows "invalid parameter "commits": " or "invalid parameter "file": ".

patch-what-to-export = say what to export: commits, from (with to), staged or worktree
patch-one-source = give one of commits, from (with to), staged and worktree
patch-where-to-write = say where to write: dir or file
patch-one-destination = give one of dir and file
