``pxar`` is a command-line utility for creating and manipulating archives in the
:ref:`pxar-format`.
It is inspired by `casync file archive format
<http://0pointer.net/blog/casync-a-tool-for-distributing-file-system-images.html>`_,
which caters to a similar use-case.
The ``.pxar`` format is adapted to fulfill the specific needs of the
`Proxmox Backup`_ Server, for example, efficient storage of hard links.
The format is designed to reduce the required storage on the server by
achieving a high level of deduplication.

Creating an Archive
^^^^^^^^^^^^^^^^^^^

Run the following command to create an archive of a folder named ``source``:

.. code-block:: console

    # pxar create archive.pxar /path/to/source

This will create a new archive called ``archive.pxar`` with the contents of the
``source`` folder.

.. NOTE:: ``pxar`` will not overwrite any existing archives. If an archive with
    the same name is already present in the target folder, the creation will
    fail.

By default, ``pxar`` will skip certain mount points and will not follow device
boundaries. This design decision is based on the primary use case of creating
archives for backups. It makes sense to ignore the contents of certain
temporary or system specific files in a backup.
To alter this behavior and follow device boundaries, use the
``--all-file-systems`` flag.

It is possible to exclude certain files and/or folders from the archive by
passing the ``--exclude`` parameter with ``gitignore``\-style match patterns.

For example, you can exclude all files ending in ``.txt`` from the archive
by running:

.. code-block:: console

    # pxar create archive.pxar /path/to/source --exclude '**/*.txt'

Be aware that the shell itself will try to expand glob patterns before invoking
``pxar``. In order to avoid this, all globs have to be quoted correctly.

It is possible to pass the ``--exclude`` parameter multiple times, in order to
match more than one pattern. This allows you to use more complex
file inclusion/exclusion behavior. However, it is recommended to use
``.pxarexclude`` files instead for such cases.

For example you might want to exclude all ``.txt`` files except a specific
one from the archive. This would be achieved via the negated match pattern,
prefixed by ``!``.  All the glob patterns are relative to the ``source``
directory.

.. code-block:: console

    # pxar create archive.pxar /path/to/source --exclude '**/*.txt' --exclude '!/folder/file.txt'

.. NOTE:: The order of the glob match patterns matters, as later ones override
   earlier ones. Permutations of the same patterns lead to different results.

``pxar`` will store the list of glob match patterns passed as parameters via the
command line, in a file called ``.pxarexclude-cli``, at the root of the archive.
If a file with this name is already present in the source folder during archive
creation, this file is not included in the archive, and the file containing the
new patterns is added to the archive instead. The original file is not altered.

A more convenient and persistent way to exclude files from the archive is by
placing the glob match patterns in ``.pxarexclude`` files.
It is possible to create and place these files in any directory of the filesystem
tree.
These files must contain one pattern per line, and later patterns override
earlier ones.
The patterns control file exclusions of files present within the given directory
or further below it in the tree.
The behavior is the same as described in :ref:`client_creating_backups`.

Extracting an Archive
^^^^^^^^^^^^^^^^^^^^^

An existing archive, ``archive.pxar``, is extracted to a ``target`` directory
with the following command:

.. code-block:: console

    # pxar extract archive.pxar /path/to/target

If no target is provided, the contents of the archive is extracted to the current
working directory.

In order to restore only parts of an archive, single files, and/or folders,
it is possible to pass the corresponding glob match patterns as additional
parameters or to use the patterns stored in a file:

.. code-block:: console

    # pxar extract etc.pxar /restore/target/etc --pattern '**/*.conf'

The above example restores all ``.conf`` files encountered in any of the
sub-folders in the archive ``etc.pxar`` to the target ``/restore/target/etc``.
A path to the file containing match patterns can be specified using the
``--files-from`` parameter.

List the Contents of an Archive
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

To display the files and directories contained in an archive ``archive.pxar``,
run the following command:

.. code-block:: console

    # pxar list archive.pxar

This displays the full path of each file or directory with respect to the
archive's root.

Mounting an Archive
^^^^^^^^^^^^^^^^^^^

``pxar`` allows you to mount and inspect the contents of an archive via _`FUSE`.
In order to mount an archive named ``archive.pxar`` to the mount point ``/mnt``,
run the command:

.. code-block:: console

    # pxar mount archive.pxar /mnt

Once the archive is mounted, you can access its content under the given
mount point.

.. code-block:: console

    # cd /mnt
    # ls
    bin   dev  home  lib32  libx32      media  opt   root  sbin  sys  usr
    boot  etc  lib   lib64  lost+found  mnt    proc  run   srv   tmp  var

