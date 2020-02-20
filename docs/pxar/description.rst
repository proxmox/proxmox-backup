Description
^^^^^^^^^^^

``pxar`` is a command line utility used to create and manipulate archives in the
:ref:`pxar-format`.
It is inspired by `casync file archive format
<http://0pointer.net/blog/casync-a-tool-for-distributing-file-system-images.html>`_,
which has a similar use-case.
The ``.pxar`` format is adapted to fulfill the specific needs of the proxmox
backup server, for example efficient storage of hardlinks.
The format is designed to reduce storage space needed on the server by achieving
high de-duplication.

Creating an Archive
^^^^^^^^^^^^^^^^^^^

Run the following command to create an archive of a folder named ``source``:

.. code-block:: console

    # pxar create archive.pxar source

This will create a new archive called ``archive.pxar`` from the contents of the
``source`` folder.

.. NOTE:: ``pxar`` will not overwrite any existing archives. If an archive with
    the same name is already present in the target folder, the creation will
    fail.

By default, ``pxar`` will skip certain mountpoints and not follow device
boundaries. This design decision is based on the primary use case of creating
archives for backups, where it makes no sense to store the content of certain
temporary or system specific files.
In order to alter this behavior and follow device boundaries, use the
``--all-file-systems`` flag.

It is possible to exclude certain files and/or folders from the archive by
passing glob match patterns as additional parameters. Whenever a file is matched
by one of the patterns, you will get a warning saying that this file is skipped
and therefore not included in the archive.

For example, you can exclude all files ending in ``.txt`` from the archive
by running:

.. code-block:: console

    # pxar create archive.pxar source '**/*.txt'

Be aware that the shell itself will try to expand all of the glob patterns before
invoking ``pxar``.
In order to avoid this, all globs have to be quoted correctly.

It is also possible to pass a list of match pattern to fulfill more complex
file exclusion/inclusion behavior, although it is recommended to use the
``.pxarexclude`` files instead for such cases.

For example you might want to exclude all ``.txt`` files except for a specific
one from the archive. This is achieved via the negated match pattern, prefixed
by ``!``.
All the glob pattern are relative to the ``source`` directory.

.. code-block:: console

    # pxar create archive.pxar source '**/*.txt' '!/folder/file.txt'

.. NOTE:: The order of the glob match patterns matters as later ones win over
    previous ones. Permutations of the same patterns lead to different results.

``pxar`` will store the list of glob match patterns passed as parameters via the
command line in a file called ``.pxarexclude-cli`` and store it at the root of
the archive.
If a file with this name is already present in the source folder during archive
creation, this file is not included in the archive and the file containing the
new patterns is added to the archive instead, the original file is not altered.

A more convenient and persistent way to exclude files from the archive is by
placing the glob match patterns in ``.pxarexclude`` files.
It is possible to create and place these files in any directory of the filesystem
tree.
These files must contain one pattern per line, again later patterns win over
previous ones.
The patterns control file exclusion of files present within the given directory
or further below it in the tree.
The behaviour is the same as described in :ref:`creating-backups`.

Extracting an Archive
^^^^^^^^^^^^^^^^^^^^^

An existing archive ``archive.pxar`` is extracted to a ``target`` directory
with the following command:

.. code-block:: console

    # pxar extract archive.pxar --target target

If no target is provided, the content of the archive is extracted to the current
working directory.

In order to restore only part of an archive or single files and/or folders,
it is possible to pass the corresponding glob match patterns as additional
parameters or use the patterns stored in a file:

.. code-block:: console

    # pxar extract etc.pxar '**/*.conf' --target /restore/target/etc

The above example restores all ``.conf`` files encountered in any of the
sub-folders in the archive ``etc.pxar`` to the target ``/restore/target/etc``.
A path to the file containing match patterns can be specified using the
``--files-from`` parameter.

List the Content of an Archive
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

To display the files and directories contained in an archive ``archive.pxar``,
run the following command:

.. code-block:: console

    # pxar list archive.pxar

This displays the full path of each file or directory with respect to the
archives root.

Mounting an Archive
^^^^^^^^^^^^^^^^^^^

``pxar`` allows you to mount and inspect the contents of an archive via _`FUSE`.
In order to mount an archive named ``archive.pxar`` to the mountpoint ``mnt``,
run the command:

.. code-block:: console

    # pxar mount archive.pxar /mnt

Once the archive is mounted, you can access its content under the given
mountpoint.

.. code-block:: console

    # cd /mnt
    # ls
    bin   dev  home  lib32  libx32      media  opt   root  sbin  sys  usr
    boot  etc  lib   lib64  lost+found  mnt    proc  run   srv   tmp  var

