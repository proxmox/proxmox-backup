File Formats
============

.. _pxar-format:

Proxmox File Archive Format (``.pxar``)
---------------------------------------

.. graphviz:: pxar-format-overview.dot


.. _data-blob-format:

Data Blob Format
----------------

The data blob format is used to store small binary data. The magic number decides the exact format:

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

Compression algorithm is ``zstd``. Encryption cipher is ``AES_256_GCM``.

Unencrypted blobs use the following format:

.. list-table::
   :widths: auto

   * - ``MAGIC: [u8; 8]``
   * - ``CRC32: [u8; 4]``
   * - ``Data: (max 16MiB)``

Encrypted blobs additionally contains a 16 byte IV, followed by a 16
byte Authenticated Encyryption (AE) tag, followed by the encrypted
data:

.. list-table::

   * - ``MAGIC: [u8; 8]``
   * - ``CRC32: [u8; 4]``
   * - ``ÌV: [u8; 16]``
   * - ``TAG: [u8; 16]``
   * - ``Data: (max 16MiB)``
