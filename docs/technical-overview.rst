.. _tech_design_overview:

Technical Overview
==================

Datastores
----------

A Datastore is the logical place where :ref:`Backup Snapshots
<term_backup_snapshot>` and their chunks are stored. Snapshots consist of a
manifest, blobs, and dynamic- and fixed-indexes (see :ref:`terms`), and are
stored in the following directory structure:

 <datastore-root>/<type>/<id>/<time>/

The deduplication of datastores is based on reusing chunks, which are
referenced by the indexes in a backup snapshot. This means that multiple
indexes can reference the same chunks, reducing the amount of space needed to
contain the data (even across backup snapshots).

Snapshots
---------

A Snapshot is the collection of manifest, blobs and indexes that represent
a backup. When a client creates a snapshot, it can upload blobs (single files
which are not chunked, e.g. the client log), or one or more indexes
(fixed or dynamic).

When uploading an index, the client first has to read the source data, chunk it
and send the data as chunks with their identifying checksum to the server.

If there is a previous Snapshot in the backup group, the client can first
download the chunk list of the previous Snapshot. If it detects a chunk that
already exists on the server, it can send only the checksum instead of data
and checksum. This way the actual upload of Snapshots is incremental while
each Snapshot references all chunks and is thus a full backup.

After uploading all data, the client has to signal to the server that the
backup is finished. If that is not done before the connection closes, the
server will remove the unfinished snapshot.

Chunks
------

A chunk is some (possibly encrypted) data with a CRC-32 checksum at the end and
a type marker at the beginning. It is identified by the SHA-256 checksum of its
content.

To generate such chunks, backup data is split either into fixed-size or
dynamically sized chunks. The same content will be hashed to the same checksum.

The chunks of a datastore are found in

 <datastore-root>/.chunks/

This chunk directory is further subdivided by the first four bytes of the
chunk's checksum, so a chunk with the checksum

 a342e8151cbf439ce65f3df696b54c67a114982cc0aa751f2852c2f7acc19a8b

lives in

 <datastore-root>/.chunks/a342/

This is done to reduce the number of files per directory, as having many files
per directory can be bad for file system performance.

These chunk directories ('0000'-'ffff') will be preallocated when a datastore
is created.

Fixed-Sized Chunks
^^^^^^^^^^^^^^^^^^

For block based backups (like VMs), fixed-sized chunks are used. The content
(disk image), is split into chunks of the same length (typically 4 MiB).

This works very well for VM images, since the file system on the guest most
often tries to allocate files in contiguous pieces, so new files get new
blocks, and changing existing files changes only their own blocks.

As an optimization, VMs in `Proxmox VE`_ can make use of 'dirty bitmaps', which
can track the changed blocks of an image. Since these bitmaps are also a
representation of the image split into chunks, there is a direct relation
between the dirty blocks of the image and chunks which need to be uploaded.
Thus, only modified chunks of the disk need to be uploaded to a backup.

Since the image is always split into chunks of the same size, unchanged blocks
will result in identical checksums for those chunks, so such chunks do not need
to be backed up again. This way storage snapshots are not needed to find the
changed blocks.

For consistency, `Proxmox VE`_ uses a QEMU internal snapshot mechanism, that
does not rely on storage snapshots either.

Dynamically Sized Chunks
^^^^^^^^^^^^^^^^^^^^^^^^

When working with file-based systems rather than block-based systems,
using fixed-sized chunks is not a good idea, since every time a file
would change in size, the remaining data would be shifted around,
resulting in many chunks changing and the amount of deduplication being reduced.

To improve this, `Proxmox Backup`_ Server uses dynamically sized chunks
instead. Instead of splitting an image into fixed sizes, it first generates a
consistent file archive (:ref:`pxar <pxar-format>`) and uses a rolling hash
over this on-the-fly generated archive to calculate chunk boundaries.

We use a variant of Buzhash which is a cyclic polynomial algorithm. It works
by continuously calculating a checksum while iterating over the data, and on
certain conditions, it triggers a hash boundary.

Assuming that most files on the system that is to be backed up have not
changed, eventually the algorithm triggers the boundary on the same data as a
previous backup, resulting in chunks that can be reused.

Encrypted Chunks
^^^^^^^^^^^^^^^^

Encrypted chunks are a special case. Both fixed- and dynamically sized chunks
can be encrypted, and they are handled in a slightly different manner than
normal chunks.

The hashes of encrypted chunks are calculated not with the actual (encrypted)
chunk content, but with the plain-text content, concatenated with the encryption
key. This way, two chunks with the same data but encrypted with different keys
generate two different checksums and no collisions occur for multiple
encryption keys.

This is done to speed up the client part of the backup, since it only needs to
encrypt chunks that are actually getting uploaded. Chunks that exist already in
the previous backup, do not need to be encrypted and uploaded.

Caveats and Limitations
-----------------------

Notes on Hash Collisions
^^^^^^^^^^^^^^^^^^^^^^^^

Every hashing algorithm has a chance to produce collisions, meaning two (or
more) inputs generate the same checksum. For SHA-256, this chance is
negligible. To calculate the chances of such a collision, one can use the ideas
of the 'birthday problem' from probability theory. For big numbers, this is
actually unfeasible to calculate with regular computers, but there is a good
approximation:

.. math::

 p(n, d) = 1 - e^{-n^2/(2d)}

Where `n` is the number of tries, and `d` is the number of possibilities.
For a concrete example, lets assume a large datastore of 1 PiB and an average
chunk size of 4 MiB. That means :math:`n = 268435456` tries, and :math:`d =
2^{256}` possibilities. Inserting those values in the formula from earlier you
will see that the probability of a collision in that scenario is:

.. math::

 3.1115 * 10^{-61}

For context, in a lottery game of guessing 6 numbers out of 45, the chance to
correctly guess all 6 numbers is only :math:`1.2277 * 10^{-7}`. This means the
chance of a collision is about the same as winning 13 such lottery games *in a
row*.

In conclusion, it is extremely unlikely that such a collision would occur by
accident in a normal datastore.

Additionally, SHA-256 is prone to length extension attacks, but since there is
an upper limit for how big the chunks are, this is not a problem, because a
potential attacker cannot arbitrarily add content to the data beyond that
limit.

File-Based Backup
^^^^^^^^^^^^^^^^^

Since dynamically sized chunks (for file-based backups) are created on a custom
archive format (pxar) and not over the files directly, there is no relation
between the files and chunks. This means that the Proxmox Backup Client has to
read all files again for every backup, otherwise it would not be possible to
generate a consistent, independent pxar archive where the original chunks can be
reused. Note that in spite of this, only new or changed chunks will be uploaded.

Verification of Encrypted Chunks
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

For encrypted chunks, only the checksum of the original (plaintext) data is
available, making it impossible for the server (without the encryption key) to
verify its content against it. Instead only the CRC-32 checksum gets checked.

Troubleshooting
---------------

Index files(*.fidx*, *.didx*) contain information about how to rebuild a file.
More precisely, they contain an ordered list of references to the chunks that
the original file was split into. If there is something wrong with a snapshot,
it might be useful to find out which chunks are referenced in it, and check
whether they are present and intact. The ``proxmox-backup-debug`` command-line
tool can be used to inspect such files and recover their contents. For example,
to get a list of the referenced chunks of a *.fidx* index:

.. code-block:: console

    # proxmox-backup-debug inspect file drive-scsi0.img.fidx

The same command can be used to inspect *.blob* files. Without the ``--decode``
parameter, just the size and the encryption type, if any, are printed. If
``--decode`` is set, the blob file is decoded into the specified file ('-' will
decode it directly to stdout).

The following example would print the decoded contents of
`qemu-server.conf.blob`. If the file you're trying to inspect is encrypted, a
path to the key file must be provided using ``--keyfile``.

.. code-block:: console

    # proxmox-backup-debug inspect file qemu-server.conf.blob --decode -

You can also check in which index files a specific chunk file is referenced
with:

.. code-block:: console

    # proxmox-backup-debug inspect chunk b531d3ffc9bd7c65748a61198c060678326a431db7eded874c327b7986e595e0 --reference-filter /path/in/a/datastore/directory

Here ``--reference-filter`` specifies where index files should be searched. This
can be an arbitrary path. If, for some reason, the filename of the chunk was
changed, you can explicitly specify the digest using ``--digest``. By default, the
chunk filename is used as the digest to look for. If no ``--reference-filter``
is specified, it will only print the CRC and encryption status of the chunk. You
can also decode chunks, by setting the ``--decode`` flag. If the chunk is
encrypted, a ``--keyfile`` must be provided, in order to decode it.

Restore without a Running Proxmox Backup Server
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

It's possible to restore specific files from a snapshot, without a running
`Proxmox Backup`_ Server instance, using the ``recover`` subcommand, provided
you have access to the intact index and chunk files. Note that you also need the
corresponding key file if the backup was encrypted.

.. code-block:: console

    # proxmox-backup-debug recover index drive-scsi0.img.fidx /path/to/.chunks

In the above example, the `/path/to/.chunks` argument is the path to the
directory that contains the chunks, and `drive-scsi0.img.fidx` is the index file
of the file you'd like to restore. Both paths can be absolute or relative. With
``--skip-crc``, it's possible to disable the CRC checks of the chunks. This
will speed up the process slightly and allow for trying to restore (partially)
corrupt chunks. It's recommended to always try without the skip-CRC option
first.

