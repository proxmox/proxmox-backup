Command Syntax
==============

.. NOTE:: Logging verbosity for the command-line tools can be controlled with the
    ``PBS_LOG`` (for ``pxar``: ``PXAR_LOG``) environment variable. Possible values are `off`,
    `error`, `warn`, `info`, `debug` and `trace` with `info` being the default.

``proxmox-backup-client``
-------------------------

.. include:: proxmox-backup-client/synopsis.rst


Catalog Shell Commands
~~~~~~~~~~~~~~~~~~~~~~

The following commands are available in an interactive restore shell:

.. code-block:: console

    proxmox-backup-client shell <snapshot> <name.pxar>


.. include:: proxmox-backup-client/catalog-shell-synopsis.rst


``proxmox-backup-manager``
--------------------------

.. include:: proxmox-backup-manager/synopsis.rst


``proxmox-tape``
----------------

.. include:: proxmox-tape/synopsis.rst

``pmt``
-------

.. include:: pmt/options.rst

....

.. include:: pmt/synopsis.rst


``pmtx``
--------

.. include:: pmtx/synopsis.rst


``pxar``
--------

.. include:: pxar/synopsis.rst


``proxmox-file-restore``
------------------------
.. include:: proxmox-file-restore/synopsis.rst


``proxmox-backup-debug``
------------------------
.. include:: proxmox-backup-debug/synopsis.rst
