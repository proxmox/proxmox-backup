File Formats
============

.. _pxar-format:

Proxmox File Archive Format (``.pxar``)
---------------------------------------

.. graphviz:: pxar-format-overview.dot


.. _data-blob-format:

Data Blob Format (``.blob``)
----------------------------

The data blob format is used to store small binary data. The magic number
decides the exact format:

.. list-table::
   :widths: auto

   * - ``[66, 171, 56, 7, 190, 131, 112, 161]``
     - unencrypted
     - uncompressed
   * - ``[49, 185, 88, 66, 111, 182, 163, 127]``
     - unencrypted
     - compressed
   * - ``[123, 103, 133, 190, 34, 45, 76, 240]``
     - encrypted
     - uncompressed
   * - ``[230, 89, 27, 191, 11, 191, 216, 11]``
     - encrypted
     - compressed

The compression algorithm used is ``zstd``. The encryption cipher is
``AES_256_GCM``.

Unencrypted blobs use the following format:

.. list-table::
   :widths: auto

   * - ``MAGIC: [u8; 8]``
   * - ``CRC32: [u8; 4]``
   * - ``Data: (max 16MiB)``

Encrypted blobs additionally contain a 16 byte initialization vector (IV),
followed by a 16 byte authenticated encryption (AE) tag, followed by the
encrypted data:

.. list-table::

   * - ``MAGIC: [u8; 8]``
   * - ``CRC32: [u8; 4]``
   * - ``IV: [u8; 16]``
   * - ``TAG: [u8; 16]``
   * - ``Data: (max 16MiB)``


.. _fixed-index-format:

Fixed Index Format  (``.fidx``)
-------------------------------

All numbers are stored as little-endian.

.. list-table::

   * - ``MAGIC: [u8; 8]``
     - ``[47, 127, 65, 237, 145, 253, 15, 205]``
   * - ``uuid: [u8; 16]``,
     - Unique ID
   * - ``ctime: i64``,
     - Creation Time (epoch)
   * - ``index_csum: [u8; 32]``,
     - SHA-256 over the index (without header) ``SHA256(digest1||digest2||...)``
   * - ``size: u64``,
     - Image size
   * - ``chunk_size: u64``,
     - Chunk size
   * - ``reserved: [u8; 4016]``,
     - Overall header size is one page (4096 bytes)
   * - ``digest1: [u8; 32]``
     - First chunk digest
   * - ``digest2: [u8; 32]``
     - Second chunk digest
   * - ...
     - Next chunk digest ...


.. _dynamic-index-format:

Dynamic Index Format (``.didx``)
--------------------------------

All numbers are stored as little-endian.

.. list-table::

   * - ``MAGIC: [u8; 8]``
     - ``[28, 145, 78, 165, 25, 186, 179, 205]``
   * - ``uuid: [u8; 16]``,
     - Unique ID
   * - ``ctime: i64``,
     - Creation Time (epoch)
   * - ``index_csum: [u8; 32]``,
     - SHA-256 over the index (without header) ``SHA256(offset1||digest1||offset2||digest2||...)``
   * - ``reserved: [u8; 4032]``,
     - Overall header size is one page (4096 bytes)
   * - ``offset1: u64``
     - End of first chunk
   * - ``digest1: [u8; 32]``
     - First chunk digest
   * - ``offset2: u64``
     - End of second chunk
   * - ``digest2: [u8; 32]``
     - Second chunk digest
   * - ...
     - Next chunk offset/digest
