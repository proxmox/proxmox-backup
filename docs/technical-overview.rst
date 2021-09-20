.. _tech_design_overview:

Technical Overview
==================

Datastores
----------

A Datastore is the logical place where :ref:`Backup Snapshots
<term_backup_snapshot>` and their chunks are stored. Snapshots consist of a
manifest, blobs, dynamic- and fixed-indexes (see :ref:`terms`), and are
stored in the following directory structure:

 <datastore-root>/<type>/<id>/<time>/

The deduplication of datastores is based on reusing chunks, which are
referenced by the indexes in a backup snapshot. This means that multiple
indexes can reference the same chunks, reducing the amount of space needed to
contain the data (even across backup snapshots).

Chunks
------

A chunk is some (possibly encrypted) data with a CRC-32 checksum at the end and
a type marker at the beginning. It is identified by the SHA-256 checksum of its
content.

To generate such chunks, backup data is split either into fixed-size or
dynamically sized chunks. The same content will be hashed to the same checksum.

The chunks of a datastore are found in

 <datastore-root>/.chunks/

This chunk directory is further subdivided by the first four byte of the chunks
checksum, so the chunk with the checksum

 a342e8151cbf439ce65f3df696b54c67a114982cc0aa751f2852c2f7acc19a8b

lives in

 <datastore-root>/.chunks/a342/

This is done to reduce the number of files per directory, as having many files
per directory can be bad for file system performance.

These chunk directories ('0000'-'ffff') will be preallocated when a datastore
is created.

Fixed-sized Chunks
^^^^^^^^^^^^^^^^^^

For block based backups (like VMs), fixed-sized chunks are used. The content
(disk image), is split into chunks of the same length (typically 4 MiB).

This works very well for VM images, since the file system on the guest most
often tries to allocate files in contiguous pieces, so new files get new
blocks, and changing existing files changes only their own blocks.

As an optimization, VMs in `Proxmox VE`_ can make use of 'dirty bitmaps', which
can track the changed blocks of an image. Since these bitmap are also a
representation of the image split into chunks, there is a direct relation
between dirty blocks of the image and chunks which need to get uploaded, so
only modified chunks of the disk have to be uploaded for a backup.

Since the image is always split into chunks of the same size, unchanged blocks
will result in identical checksums for those chunks, so such chunks do not need
to be backed up again. This way storage snapshots are not needed to find the
changed blocks.

For consistency, `Proxmox VE`_ uses a QEMU internal snapshot mechanism, that
does not rely on storage snapshots either.

Dynamically sized Chunks
^^^^^^^^^^^^^^^^^^^^^^^^

If one does not want to backup block-based systems but rather file-based
systems, using fixed-sized chunks is not a good idea, since every time a file
would change in size, the remaining data gets shifted around and this would
result in many chunks changing, reducing the amount of deduplication.

To improve this, `Proxmox Backup`_ Server uses dynamically sized chunks
instead. Instead of splitting an image into fixed sizes, it first generates a
consistent file archive (:ref:`pxar <pxar-format>`) and uses a rolling hash
over this on-the-fly generated archive to calculate chunk boundaries.

We use a variant of Buzhash which is a cyclic polynomial algorithm.  It works
by continuously calculating a checksum while iterating over the data, and on
certain conditions it triggers a hash boundary.

Assuming that most files of the system that is to be backed up have not
changed, eventually the algorithm triggers the boundary on the same data as a
previous backup, resulting in chunks that can be reused.

Encrypted Chunks
^^^^^^^^^^^^^^^^

Encrypted chunks are a special case. Both fixed- and dynamically sized chunks
can be encrypted, and they are handled in a slightly different manner than
normal chunks.

The hashes of encrypted chunks are calculated not with the actual (encrypted)
chunk content, but with the plain-text content concatenated with the encryption
key. This way, two chunks of the same data encrypted with different keys
generate two different checksums and no collisions occur for multiple
encryption keys.

This is done to speed up the client part of the backup, since it only needs to
encrypt chunks that are actually getting uploaded. Chunks that exist already in
the previous backup, do not need to be encrypted and uploaded.

Caveats and Limitations
-----------------------

Notes on hash collisions
^^^^^^^^^^^^^^^^^^^^^^^^

Every hashing algorithm has a chance to produce collisions, meaning two (or
more) inputs generate the same checksum. For SHA-256, this chance is
negligible.  To calculate such a collision, one can use the ideas of the
'birthday problem' from probability theory. For big numbers, this is actually
infeasible to calculate with regular computers, but there is a good
approximation:

.. math::

 p(n, d) = 1 - e^{-n^2/(2d)}

Where `n` is the number of tries, and `d` is the number of possibilities.
For a concrete example lets assume a large datastore of 1 PiB, and an average
chunk size of 4 MiB. That means :math:`n = 268435456` tries, and :math:`d =
2^{256}` possibilities. Inserting those values in the formula from earlier you
will see that the probability of a collision in that scenario is:

.. math::

 3.1115 * 10^{-61}

For context, in a lottery game of guessing 6 out of 45, the chance to correctly
guess all 6 numbers is only :math:`1.2277 * 10^{-7}`, that means the chance of
a collision is about the same as winning 13 such lotto games *in a row*.

In conclusion, it is extremely unlikely that such a collision would occur by
accident in a normal datastore.

Additionally, SHA-256 is prone to length extension attacks, but since there is
an upper limit for how big the chunk are, this is not a problem, since a
potential attacker cannot arbitrarily add content to the data beyond that
limit.

File-based Backup
^^^^^^^^^^^^^^^^^

Since dynamically sized chunks (for file-based backups) are created on a custom
archive format (pxar) and not over the files directly, there is no relation
between files and the chunks. This means  that the Proxmox Backup client has to
read all files again for every backup, otherwise it would not be possible to
generate a consistent independent pxar archive where the original chunks can be
reused. Note that there will be still only new or change chunks be uploaded.

Verification of encrypted chunks
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

For encrypted chunks, only the checksum of the original (plaintext) data is
available, making it impossible for the server (without the encryption key), to
verify its content against it. Instead only the CRC-32 checksum gets checked.

Troubleshooting
---------------

Index files(.fidx, .didx) contain information about how to rebuild a file, more precisely, they
contain an ordered list of references to the chunks the original file was split up
in. If there is something wrong with a snapshot it might be useful to find out
which chunks are referenced in this specific snapshot, and check wheather all of
them are present and intact. The command for getting the list of referenced chunks
could look something like this:

.. code-block:: console

    # proxmox-backup-debug inspect file drive-scsi0.img.fidx

The same command can be used to look at .blob file, without ``--decode`` just the size
and the encryption type, if any, is printed. If ``--decode`` is set the blob file is
decoded into the specified file('-' will decode it directly into stdout).

.. code-block:: console

    # proxmox-backup-debug inspect file qemu-server.conf.blob --decode -

would print the decoded contents of `qemu-server.conf.blob`. If the file you're
trying to inspect is encrypted, a path to the keyfile has to be provided using
``--keyfile``.

Checking in which index files a specific chunk file is referenced can be done
with:

.. code-block:: console

    # proxmox-backup-debug inspect chunk b531d3ffc9bd7c65748a61198c060678326a431db7eded874c327b7986e595e0 --reference-filter ../../

Here ``--reference-filter`` specifies where index files should be searched, this can be an
arbitrary path. If, for some reason, the filename of the chunk was changed you can explicitly
specify the digest using ``--digest``, by default the chunk filename is used as the digest
to look for. Specifying no ``--reference-filter`` will just print the CRC and encryption status
of the chunk. You can also decode chunks, to do so ``--decode`` has to be set. If the chunk
is encrypted a ``--keyfile`` has to be provided for decoding.

Restore without a running PBS
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

It is possible to restore snapshots even without a running PBS, assuming you have
access to the index and chunk files, if encrypted you'll also need the keyfile
it was encrypted with.

.. code-block:: console

    # proxmox-backup-debug recover index drive-scsi0.img.fidx ../../../.chunks

where `../../../.chunks` is the path to the directory that contains contains the
chunks and `drive-scsi0.img.fidx` is the index-file of the file you'd lile to
restore. Both paths can be absolute or relative. With ``--skip-crc`` it is possible to
disable the crc checks of the chunks, this will speed up the process, however should
probably only be used for testing.

