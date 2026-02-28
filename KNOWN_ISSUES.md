# Known Issues

## arcforge-room: compile errors in `RoomManager`

**Crate:** `arcforge-room`
**File:** `crates/arcforge-room/src/manager.rs`
**Severity:** Blocking (crate does not compile)
**Found:** 2026-02-28 during tick crate audit

### Errors

1. **Use of moved value `sender` in `join_or_create`** (line 228)

   `join_or_create` iterates over rooms trying to join one, passing `sender`
   by value into `handle.join()` inside the loop. If the first attempt fails,
   `sender` has already been moved and can't be used on the next iteration or
   the fallback `handle.join()` after the loop.

   ```
   error[E0382]: use of moved value: `sender`
     --> crates/arcforge-room/src/manager.rs:228:32
   ```

   **Fix:** Either clone `sender` before the loop (it's an
   `mpsc::UnboundedSender` which is cheap to clone), or restructure to only
   call `join` once on the chosen room.

2. **Missing `Clone` bound on `RoomHandle<G>`** (line 185)

   `room_ids()` (or a similar method) calls `.cloned()` on
   `self.rooms.values()`, but `RoomHandle<G>` only derives `Clone` when `G:
   Clone`. The `GameLogic` trait doesn't require `Clone`, so the bound is
   unsatisfied.

   ```
   error[E0277]: the trait bound `G: Clone` is not satisfied
     --> crates/arcforge-room/src/manager.rs:185:29
   ```

   **Fix:** Either add `Clone` as a supertrait bound on `GameLogic`, or
   avoid cloning `RoomHandle` (e.g. return `RoomId`s by collecting keys
   instead of values).
