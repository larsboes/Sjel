# entities-google-sync

The scheduled Google Contacts sync for [`crates/entities`](../../crates/entities/). Every 12 hours it
runs `entities-server sync-google`, which reads contacts with the `contacts.readonly` scope and
applies them to the entity store. The token is in the overlay's `entities.env`. Nothing is written
back to Google.

The job is separate for the same reason as [`entities-sync`](../entities-sync/): `entities` is an
always-on service, and a manifest cannot declare both `autostart` and `schedule`.

The sync rule is in `crates/entities/src/sync.rs`. A value edited elsewhere is not
overwritten, and a contact whose person the operator deleted is not created again. A contact with
no name is skipped, and the run prints how many it skipped.

Run it by hand, without writing:

```
target/release/entities-server sync-google --dry-run
```
