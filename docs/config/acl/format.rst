This file contains the access control list for the Proxmox Backup
Server API.

Each line starts with ``acl:``, followed by 4 additional values
separated by colon.

:propagate: Propagate permissions down the hierarchy

:path: The object path

:User/Token: List of users and tokens

:Role: List of assigned roles

Here is an example list::

   acl:1:/:root@pam!test:Admin
   acl:1:/datastore/store1:user1@pbs:DatastoreAdmin


You can use the ``proxmox-backup-manager acl`` command to manipulate
this file.
