Backup Client Usage
===================

The command-line client for `Proxmox Backup`_ Server is called
:command:`proxmox-backup-client`.

.. _client_repository:

Backup Repository Locations
---------------------------

The client uses the following format to specify a datastore repository
on the backup server (where username is specified in the form of user@realm):

  [[username@]server[:port]:]datastore

The default value for ``username`` is ``root@pam``. If no server is specified,
the default is the local host (``localhost``).

You can specify a port if your backup server is only reachable on a non-default
port (for example, with NAT and port forwarding configurations).

Note that if the server uses an IPv6 address, you have to write it with square
brackets (for example, `[fe80::01]`).

You can pass the repository with the ``--repository`` command-line option, or
by setting the ``PBS_REPOSITORY`` environment variable.

The web interface provides copyable repository text in the datastore summary
with the `Show Connection Information` button.

Below are some examples of valid repositories and their corresponding real
values:

================================ ================== ================== ===========
Example                          User               Host:Port          Datastore
================================ ================== ================== ===========
mydatastore                      ``root@pam``       localhost:8007     mydatastore
myhostname:mydatastore           ``root@pam``       myhostname:8007    mydatastore
user@pbs@myhostname:mydatastore  ``user@pbs``       myhostname:8007    mydatastore
user\@pbs!token@host:store       ``user@pbs!token`` host:8007          store
192.168.55.55:1234:mydatastore   ``root@pam``       192.168.55.55:1234 mydatastore
[ff80::51]:mydatastore           ``root@pam``       [ff80::51]:8007    mydatastore
[ff80::51]:1234:mydatastore      ``root@pam``       [ff80::51]:1234    mydatastore
================================ ================== ================== ===========

Environment Variables
---------------------

``PBS_REPOSITORY``
  The default backup repository.

``PBS_PASSWORD``
  When set, this value is used as the password for the backup server.
  You can also set this to an API token secret.

``PBS_PASSWORD_FD``, ``PBS_PASSWORD_FILE``, ``PBS_PASSWORD_CMD``
  Like ``PBS_PASSWORD``, but read data from an open file descriptor, a file
  name or from the `stdout` of a command, respectively. The first defined
  environment variable from the order above is preferred.

``PBS_ENCRYPTION_PASSWORD``
  When set, this value is used to access the secret encryption key (if
  protected by password).

``PBS_ENCRYPTION_PASSWORD_FD``, ``PBS_ENCRYPTION_PASSWORD_FILE``, ``PBS_ENCRYPTION_PASSWORD_CMD``
  Like ``PBS_ENCRYPTION_PASSWORD``, but read data from an open file descriptor,
  a file name or from the `stdout` of a command, respectively. The first
  defined environment variable from the order above is preferred.

``PBS_FINGERPRINT``
  When set, this value is used to verify the server certificate (only used if
  the system CA certificates cannot validate the certificate).

``ALL_PROXY``
  When set, the client uses the specified HTTP proxy for all connections to the
  backup server. Currently only HTTP proxies are supported. Valid proxy
  configurations have the following format:
  `[http://][user:password@]<host>[:port]`. Default `port` is 1080, if not
  otherwise specified.


.. Note:: The recommended solution for shielding hosts is using tunnels such as
   wireguard, instead of using an HTTP proxy.


.. Note:: Passwords must be valid UTF-8 and may not contain newlines. For your
   convenience, Proxmox Backup Server only uses the first line as password, so
   you can add arbitrary comments after the first newline.


Output Format
-------------

.. include:: output-format.rst


.. _client_creating_backups:

Creating Backups
----------------

This section explains how to create a backup from within the machine. This can
be a physical host, a virtual machine, or a container. Such backups may contain
file and image archives. There are no restrictions in this case.

.. Note:: If you want to backup virtual machines or containers on Proxmox VE,
   see :ref:`pve-integration`.

For the following example, you need to have a backup server set up, have working
credentials, and know the repository name.
In the following examples, we use ``backup-server:store1``.

.. code-block:: console

  # proxmox-backup-client backup root.pxar:/ --repository backup-server:store1
  Starting backup: host/elsa/2019-12-03T09:35:01Z
  Client name: elsa
  skip mount point: "/boot/efi"
  skip mount point: "/dev"
  skip mount point: "/run"
  skip mount point: "/sys"
  Uploaded 12129 chunks in 87 seconds (564 MB/s).
  End Time: 2019-12-03T10:36:29+01:00

This will prompt you for a password, then upload a file archive named
``root.pxar`` containing all the files in the ``/`` directory.

.. Caution:: Please note that proxmox-backup-client does not
   automatically include mount points. Instead, you will see a short
   ``skip mount point`` message for each of them. The idea is to
   create a separate file archive for each mounted disk. You can
   explicitly include them using the ``--include-dev`` option
   (i.e. ``--include-dev /boot/efi``). You can use this option
   multiple times for each mount point that should be included.

The ``--repository`` option can get quite long and is used by all commands. You
can avoid having to enter this value by setting the environment variable
``PBS_REPOSITORY``. Note that if you would like this to remain set over
multiple sessions, you should instead add the below line to your ``.bashrc``
file.

.. code-block:: console

  # export PBS_REPOSITORY=backup-server:store1

After this, you can execute all commands without having to specify the
``--repository`` option.

A single backup is allowed to contain more than one archive. For example, if
you want to back up two disks mounted at ``/mnt/disk1`` and ``/mnt/disk2``:

.. code-block:: console

  # proxmox-backup-client backup disk1.pxar:/mnt/disk1 disk2.pxar:/mnt/disk2

This creates a backup of both disks.

If you want to use a namespace for the backup target, you can add the `--ns`
parameter:

.. code-block:: console

  # proxmox-backup-client backup disk1.pxar:/mnt/disk1 disk2.pxar:/mnt/disk2 --ns a/b/c

The backup command takes a list of backup specifications, which include the
archive name on the server, the type of the archive, and the archive source at
the client. The format is:

    <archive-name>.<type>:<source-path>

Common types are ``.pxar`` for file archives and ``.img`` for block
device images. To create a backup of a block device, run the following command:

.. code-block:: console

  # proxmox-backup-client backup mydata.img:/dev/mylvm/mydata


Excluding Files/Directories from a Backup
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Sometimes it is desired to exclude certain files or directories from a backup
archive.  To tell the Proxmox Backup client when and how to ignore files and
directories, place a text file named ``.pxarexclude`` in the filesystem
hierarchy.  Whenever the backup client encounters such a file in a directory,
it interprets each line as a glob match pattern for files and directories that
are to be excluded from the backup.

The file must contain a single glob pattern per line. Empty lines and lines
starting with ``#`` (indicating a comment) are ignored.
A ``!`` at the beginning of a line reverses the glob match pattern from an
exclusion to an explicit inclusion. This makes it possible to exclude all
entries in a directory except for a few single files/subdirectories.
Lines ending in ``/`` match only on directories.
The directory containing the ``.pxarexclude`` file is considered to be the root
of the given patterns. It is only possible to match files in this directory and
its subdirectories.

.. Note:: Patterns without a leading ``/`` will also match in subdirectories,
   while patterns with a leading ``/`` will only match in the current directory.

``\`` is used to escape special glob characters.
``?`` matches any single character.
``*`` matches any character, including an empty string.
``**`` is used to match current directory and subdirectories. For example, with
the pattern ``**/*.tmp``, it would exclude all files ending in ``.tmp`` within
a directory and its subdirectories.
``[...]`` matches a single character from any of the provided characters within
the brackets. ``[!...]`` does the complementary and matches any single
character not contained within the brackets. It is also possible to specify
ranges with two characters separated by ``-``. For example, ``[a-z]`` matches
any lowercase alphabetic character, and ``[0-9]`` matches any single digit.

The order of the glob match patterns defines whether a file is included or
excluded, that is to say, later entries override earlier ones.
This is also true for match patterns encountered deeper down the directory
tree, which can override a previous exclusion.

.. Note:: Excluded directories will **not** be read by the backup client. Thus,
   a ``.pxarexclude`` file in an excluded subdirectory will have no effect.
   ``.pxarexclude`` files are treated as regular files and will be included in
   the backup archive.

For example, consider the following directory structure:

.. code-block:: console

    # ls -aR folder
    folder/:
    .  ..  .pxarexclude  subfolder0  subfolder1

    folder/subfolder0:
    .  ..  file0  file1  file2  file3  .pxarexclude

    folder/subfolder1:
    .  ..  file0  file1  file2  file3

The different ``.pxarexclude`` files contain the following:

.. code-block:: console

    # cat folder/.pxarexclude
    /subfolder0/file1
    /subfolder1/*
    !/subfolder1/file2

.. code-block:: console

    # cat folder/subfolder0/.pxarexclude
    file3

This would exclude ``file1`` and ``file3`` in ``subfolder0`` and all of
``subfolder1`` except ``file2``.

Restoring this backup will result in:

.. code-block:: console

    ls -aR restored
    restored/:
    .  ..  .pxarexclude  subfolder0  subfolder1

    restored/subfolder0:
    .  ..  file0  file2  .pxarexclude

    restored/subfolder1:
    .  ..  file2


.. _client_encryption:

Encryption
----------

Proxmox Backup supports client-side encryption with AES-256 in GCM_
mode. To set this up, you first need to create an encryption key:

.. code-block:: console

  # proxmox-backup-client key create my-backup.key
  Encryption Key Password: **************

The key is password protected by default. If you do not need this
extra protection, you can also create it without a password:

.. code-block:: console

  # proxmox-backup-client key create /path/to/my-backup.key --kdf none

Having created this key, it is now possible to create an encrypted backup, by
passing the ``--keyfile`` parameter, with the path to the key file.

.. code-block:: console

  # proxmox-backup-client backup etc.pxar:/etc --keyfile /path/to/my-backup.key
  Password: *********
  Encryption Key Password: **************
  ...

.. Note:: If you do not specify the name of the backup key, the key will be
  created in the default location
  ``~/.config/proxmox-backup/encryption-key.json``. ``proxmox-backup-client``
  will also search this location by default, in case the ``--keyfile``
  parameter is not specified.

You can avoid entering the passwords by setting the environment
variables ``PBS_PASSWORD`` and ``PBS_ENCRYPTION_PASSWORD``.


Using a Master Key to Store and Recover Encryption Keys
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

You can also use ``proxmox-backup-client key`` to create an RSA public/private
key pair, which can be used to store an encrypted version of the symmetric
backup encryption key alongside each backup and recover it later.

To set up a master key:

1. Create an encryption key for the backup:

   .. code-block:: console

     # proxmox-backup-client key create
     creating default key at: "~/.config/proxmox-backup/encryption-key.json"
     Encryption Key Password: **********
     ...

   The resulting file will be saved to ``~/.config/proxmox-backup/encryption-key.json``.

2. Create an RSA public/private key pair:

   .. code-block:: console

     # proxmox-backup-client key create-master-key
     Master Key Password: *********
     ...

   This will create two files in your current directory, ``master-public.pem``
   and ``master-private.pem``.

3. Import the newly created ``master-public.pem`` public certificate, so that
   ``proxmox-backup-client`` can find and use it upon backup.

   .. code-block:: console

     # proxmox-backup-client key import-master-pubkey /path/to/master-public.pem
     Imported public master key to "~/.config/proxmox-backup/master-public.pem"

4. With all these files in place, run a backup job:

   .. code-block:: console

     # proxmox-backup-client backup etc.pxar:/etc

   The key will be stored in your backup, under the name ``rsa-encrypted.key``.

   .. Note:: The ``--keyfile`` parameter can be excluded, if the encryption key
     is in the default path. If you specified another path upon creation, you
     must pass the ``--keyfile`` parameter.

5. To test that everything worked, you can restore the key from the backup:

   .. code-block:: console

     # proxmox-backup-client restore /path/to/backup/ rsa-encrypted.key /path/to/target

   .. Note:: You should not need an encryption key to extract this file. However, if
     a key exists at the default location
     (``~/.config/proxmox-backup/encryption-key.json``) the program will prompt
     you for an encryption key password. Simply moving ``encryption-key.json``
     out of this directory will fix this issue.

6. Then, use the previously generated master key to decrypt the file:

   .. code-block:: console

     # proxmox-backup-client key import-with-master-key /path/to/target --master-keyfile /path/to/master-private.pem --encrypted-keyfile /path/to/rsa-encrypted.key
     Master Key Password: ******
     New Password: ******
     Verify Password: ******

7. The target file will now contain the encryption key information in plain
   text. The success of this can be confirmed by passing the resulting ``json``
   file, with the ``--keyfile`` parameter, when decrypting files from the backup.

.. warning:: Without their key, backed up files will be inaccessible. Thus, you should
  keep keys ordered and in a place that is separate from the contents being
  backed up. It can happen, for example, that you back up an entire system, using
  a key on that system. If the system then becomes inaccessible for any reason
  and needs to be restored, this will not be possible, as the encryption key will be
  lost along with the broken system.

It is recommended that you keep your master key safe, but easily accessible, in
order for quick disaster recovery. For this reason, the best place to store it
is in your password manager, where it is immediately recoverable. As a backup to
this, you should also save the key to a USB flash drive and store that in a secure
place. This way, it is detached from any system, but is still easy to recover
from, in case of emergency. Finally, in preparation for the worst case scenario,
you should also consider keeping a paper copy of your master key locked away in
a safe place. The ``paperkey`` subcommand can be used to create a QR encoded
version of your master key. The following command sends the output of the
``paperkey`` command to a text file, for easy printing.

.. code-block:: console

  proxmox-backup-client key paperkey --output-format text > qrkey.txt


Restoring Data
--------------

The regular creation of backups is a necessary step in avoiding data loss. More
importantly, however, is the restoration. It is good practice to perform
periodic recovery tests to ensure that you can access the data in case of
disaster.

First, you need to find the snapshot which you want to restore. The snapshot
list command provides a list of all the snapshots on the server:

.. code-block:: console

  # proxmox-backup-client snapshot list
  ┌────────────────────────────────┬─────────────┬────────────────────────────────────┐
  │ snapshot                       │        size │ files                              │
  ╞════════════════════════════════╪═════════════╪════════════════════════════════════╡
  │ host/elsa/2019-12-03T09:30:15Z │ 51788646825 │ root.pxar catalog.pcat1 index.json │
  ├────────────────────────────────┼─────────────┼────────────────────────────────────┤
  │ host/elsa/2019-12-03T09:35:01Z │ 51790622048 │ root.pxar catalog.pcat1 index.json │
  ├────────────────────────────────┼─────────────┼────────────────────────────────────┤
  ...


.. tip:: List will by default only output the backup snapshots of the root
   namespace itself. To list backups from another namespace use the ``--ns
   <ns>`` option

You can inspect the catalog to find specific files.

.. code-block:: console

  # proxmox-backup-client catalog dump host/elsa/2019-12-03T09:35:01Z
  ...
  d "./root.pxar.didx/etc/cifs-utils"
  l "./root.pxar.didx/etc/cifs-utils/idmap-plugin"
  d "./root.pxar.didx/etc/console-setup"
  ...

The restore command lets you restore a single archive from the
backup.

.. code-block:: console

  # proxmox-backup-client restore host/elsa/2019-12-03T09:35:01Z root.pxar /target/path/

To get the contents of any archive, you can restore the ``index.json`` file in the
repository to the target path '-'. This will dump the contents to the standard output.

.. code-block:: console

  # proxmox-backup-client restore host/elsa/2019-12-03T09:35:01Z index.json -


Interactive Restores
~~~~~~~~~~~~~~~~~~~~

If you only want to restore a few individual files, it is often easier
to use the interactive recovery shell.

.. code-block:: console

  # proxmox-backup-client catalog shell host/elsa/2019-12-03T09:35:01Z root.pxar
  Starting interactive shell
  pxar:/ > ls
  bin        boot       dev        etc        home       lib        lib32
  ...

The interactive recovery shell is a minimal command-line interface that
utilizes the metadata stored in the catalog to quickly list, navigate and
search for files in a file archive.
To restore files, you can select them individually or match them with a glob
pattern.

Using the catalog for navigation reduces the overhead considerably because only
the catalog needs to be downloaded and, optionally, decrypted.
The actual chunks are only accessed if the metadata in the catalog is
insufficient or for the actual restore.

Similar to common UNIX shells, ``cd`` and ``ls`` are the commands used to change
working directory and list directory contents in the archive.
``pwd`` shows the full path of the current working directory with respect to the
archive root.

The ability to quickly search the contents of the archive is a commonly required
feature. That's where the catalog is most valuable. For example:

.. code-block:: console

  pxar:/ > find etc/**/*.txt --select
  "/etc/X11/rgb.txt"
  pxar:/ > list-selected
  etc/**/*.txt
  pxar:/ > restore-selected /target/path
  ...

This will find and print all files ending in ``.txt`` located in ``etc/`` or its
subdirectories, and add the corresponding pattern to the list for subsequent restores.
``list-selected`` shows these patterns and ``restore-selected`` finally restores
all files in the archive matching the patterns to ``/target/path`` on the local
host. This will scan the whole archive.

The ``restore`` command can be used to restore all the files contained within
the backup archive. This is most helpful when paired with the ``--pattern
<glob>`` option, as it allows you to restore all files matching a specific
pattern. For example, if you wanted to restore configuration files
located in ``/etc``, you could do the following:

.. code-block:: console

  pxar:/ > restore target/ --pattern etc/**/*.conf
  ...

The above will scan through all the directories below ``/etc`` and restore all
files ending in ``.conf``.

.. todo:: Explain interactive restore in more detail

Mounting of Archives via FUSE
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The :term:`FUSE` implementation for the pxar archive allows you to mount a
file archive as a read-only filesystem to a mount point on your host.

.. code-block:: console

  # proxmox-backup-client mount host/backup-client/2020-01-29T11:29:22Z root.pxar /mnt/mountpoint
  # ls /mnt/mountpoint
  bin   dev  home  lib32  libx32      media  opt   root  sbin  sys  usr
  boot  etc  lib   lib64  lost+found  mnt    proc  run   srv   tmp  var

This allows you to access the full contents of the archive in a seamless manner.

.. note:: As the FUSE connection needs to fetch and decrypt chunks from the
    backup server's datastore, this can cause some additional network and CPU
    load on your host, depending on the operations you perform on the mounted
    filesystem.

To unmount the filesystem, use the ``umount`` command on the mount point:

.. code-block:: console

  # umount /mnt/mountpoint

Login and Logout
----------------

The client tool prompts you to enter the login password as soon as you
want to access the backup server. The server checks your credentials
and responds with a ticket that is valid for two hours. The client
tool automatically stores that ticket and uses it for further requests
to this server.

You can also manually trigger this login/logout using the login and
logout commands:

.. code-block:: console

  # proxmox-backup-client login
  Password: **********

To remove the ticket, issue a logout:

.. code-block:: console

  # proxmox-backup-client logout


.. _changing-backup-owner:

Changing the Owner of a Backup Group
------------------------------------

By default, the owner of a backup group is the user which was used to originally
create that backup group (or in the case of sync jobs, ``root@pam``). This
means that if a user ``mike@pbs`` created a backup, another user ``john@pbs``
can not be used to create backups in that same backup group. In case you want
to change the owner of a backup, you can do so with the below command, using a
user that has ``Datastore.Modify`` privileges on the datastore.

.. code-block:: console

  # proxmox-backup-client change-owner vm/103 john@pbs

This can also be done from within the web interface, by navigating to the
`Content` section of the datastore that contains the backup group and selecting
the user icon under the `Actions` column. Common cases for this could be to
change the owner of a sync job from ``root@pam``, or to repurpose a backup
group.


.. _backup-pruning:

Pruning and Removing Backups
----------------------------

You can manually delete a backup snapshot using the ``forget`` command:

.. code-block:: console

  # proxmox-backup-client snapshot forget <snapshot>


.. caution:: This command removes all archives in this backup snapshot. They
   will be inaccessible and *unrecoverable*.

Don't forget to add the namespace ``--ns`` parameter if you want to forget a
snapshot that is contained in the root namespace:

.. code-block:: console

  # proxmox-backup-client snapshot forget <snapshot> --ns <ns>




Although manual removal is sometimes required, the ``prune``
command is normally used to systematically delete older backups. Prune lets
you specify which backup snapshots you want to keep. The
following retention options are available:

``--keep-last <N>``
  Keep the last ``<N>`` backup snapshots.

``--keep-hourly <N>``
  Keep backups for the last ``<N>`` hours. If there is more than one
  backup for a single hour, only the latest is kept.

``--keep-daily <N>``
  Keep backups for the last ``<N>`` days. If there is more than one
  backup for a single day, only the latest is kept.

``--keep-weekly <N>``
  Keep backups for the last ``<N>`` weeks. If there is more than one
  backup for a single week, only the latest is kept.

  .. note:: Weeks start on Monday and end on Sunday. The software
     uses the `ISO week date`_ system and handles weeks at
     the end of the year correctly.

``--keep-monthly <N>``
  Keep backups for the last ``<N>`` months. If there is more than one
  backup for a single month, only the latest is kept.

``--keep-yearly <N>``
  Keep backups for the last ``<N>`` years. If there is more than one
  backup for a single year, only the latest is kept.

The retention options are processed in the order given above. Each option
only covers backups within its time period. The next option does not take care
of already covered backups. It will only consider older backups.

Unfinished and incomplete backups will be removed by the prune command unless
they are newer than the last successful backup. In this case, the last failed
backup is retained.

.. code-block:: console

  # proxmox-backup-client prune <group> --keep-daily 7 --keep-weekly 4 --keep-monthly 3


You can use the ``--dry-run`` option to test your settings. This only
shows the list of existing snapshots and what actions prune would take.

.. code-block:: console

  # proxmox-backup-client prune host/elsa --dry-run --keep-daily 1 --keep-weekly 3
  ┌────────────────────────────────┬──────┐
  │ snapshot                       │ keep │
  ╞════════════════════════════════╪══════╡
  │ host/elsa/2019-12-04T13:20:37Z │    1 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-12-03T09:35:01Z │    0 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-11-22T11:54:47Z │    1 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-11-21T12:36:25Z │    0 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-11-10T10:42:20Z │    1 │
  └────────────────────────────────┴──────┘

.. note:: Neither the ``prune`` command nor the ``forget`` command free space
   in the chunk-store. The chunk-store still contains the data blocks. To free
   space you need to perform :ref:`client_garbage-collection`.

It is also possible to protect single snapshots from being pruned or deleted:

.. code-block:: console

  # proxmox-backup-client snapshot protected update <snapshot> true

This will set the protected flag on the snapshot and prevent pruning or manual
deletion of this snapshot until the flag is removed again with:

.. code-block:: console

  # proxmox-backup-client snapshot protected update <snapshot> false

When a group with a protected snapshot is deleted, only the non-protected
ones are removed, and the rest will remain.

.. note:: This flag will not be synced when using pull or sync jobs. If you
   want to protect a synced snapshot, you have to do this again manually on
   the target backup server.

.. _client_garbage-collection:

Garbage Collection
------------------

The ``prune`` command removes only the backup index files, not the data
from the datastore. This task is left to the garbage collection
command. It is recommended to carry out garbage collection on a regular basis.

The garbage collection works in two phases. In the first phase, all
data blocks that are still in use are marked. In the second phase,
unused data blocks are removed.

.. note:: This command needs to read all existing backup index files
  and touches the complete chunk-store. This can take a long time
  depending on the number of chunks and the speed of the underlying
  disks.

.. note:: The garbage collection will only remove chunks that haven't been used
   for at least one day (exactly 24h 5m). This grace period is necessary because
   chunks in use are marked by touching the chunk which updates the ``atime``
   (access time) property. Filesystems are mounted with the ``relatime`` option
   by default. This results in a better performance by only updating the
   ``atime`` property if the last access has been at least 24 hours ago. The
   downside is that touching a chunk within these 24 hours will not always
   update its ``atime`` property.

   Chunks in the grace period will be logged at the end of the garbage
   collection task as *Pending removals*.

.. code-block:: console

  # proxmox-backup-client garbage-collect
  starting garbage collection on store store2
  Start GC phase1 (mark used chunks)
  Start GC phase2 (sweep unused chunks)
  percentage done: 1, chunk count: 219
  percentage done: 2, chunk count: 453
  ...
  percentage done: 99, chunk count: 21188
  Removed bytes: 411368505
  Removed chunks: 203
  Original data bytes: 327160886391
  Disk bytes: 52767414743 (16 %)
  Disk chunks: 21221
  Average chunk size: 2486565
  TASK OK

Garbage collection can also be scheduled using ``proxmox-backup-manager`` or
from the Proxmox Backup Server's web interface.

Benchmarking
------------

The backup client also comes with a benchmarking tool. This tool measures
various metrics relating to compression and encryption speeds. If a Proxmox
Backup repository (remote or local) is specified, the TLS upload speed will get
measured too.

You can run a benchmark using the ``benchmark`` subcommand of
``proxmox-backup-client``:

.. note:: The TLS speed test is only included if a :ref:`backup server
  repository is specified <client_repository>`.

.. code-block:: console

  # proxmox-backup-client benchmark
  Uploaded 1517 chunks in 5 seconds.
  Time per request: 3309 microseconds.
  TLS speed: 1267.41 MB/s
  SHA256 speed: 2066.73 MB/s
  Compression speed: 775.11 MB/s
  Decompress speed: 1233.35 MB/s
  AES256/GCM speed: 3688.27 MB/s
  Verify speed: 783.43 MB/s
  ┌───────────────────────────────────┬─────────────────────┐
  │ Name                              │ Value               │
  ╞═══════════════════════════════════╪═════════════════════╡
  │ TLS (maximal backup upload speed) │ 1267.41 MB/s (103%) │
  ├───────────────────────────────────┼─────────────────────┤
  │ SHA256 checksum computation speed │ 2066.73 MB/s (102%) │
  ├───────────────────────────────────┼─────────────────────┤
  │ ZStd level 1 compression speed    │ 775.11 MB/s (103%)  │
  ├───────────────────────────────────┼─────────────────────┤
  │ ZStd level 1 decompression speed  │ 1233.35 MB/s (103%) │
  ├───────────────────────────────────┼─────────────────────┤
  │ Chunk verification speed          │ 783.43 MB/s (103%)  │
  ├───────────────────────────────────┼─────────────────────┤
  │ AES256 GCM encryption speed       │ 3688.27 MB/s (101%) │
  └───────────────────────────────────┴─────────────────────┘


.. note:: The percentages given in the output table correspond to a
  comparison against a Ryzen 7 2700X.

You can also pass the ``--output-format`` parameter to output stats in ``json``,
rather than the default table format.
