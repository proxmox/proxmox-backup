.. _sysadmin_traffic_control:

Traffic Control
---------------

Proxmox Backup Server allows to limit network traffic for clients
within specified source networks. The following command adds a traffic
control rule to limit all clients (network ``0.0.0.0/0``) to 100 MB/s:


.. code-block:: console

 # proxmox-backup-manager traffic-control create rule0  --network 0.0.0.0/0 \
   --rate-in 100MB --rate-out 100MB \
   --comment "Default rate limit (100MB/s) for all clients"

It is possible to restrict rules to certain time frames, for example
the company office hours:

.. code-block:: console

 # proxmox-backup-manager traffic-control update rule0  \
   --timeframe "mon..fri 8-12" \
   --timeframe "mon..fri 14:30-18"

If there are more rules, the server uses the rule with the smaller
network. For example, we can overwrite the setting for our private
network (and the server itself) with:

.. code-block:: console

 # proxmox-backup-manager traffic-control create rule1 \
   --network 192.168.2.0/24 \
   --network 127.0.0.0/8 \
   --rate-in 20GB --rate-out 20GB \
   --comment "Use 20GB/s for the local network"

.. note:: The behavior is undefined if there are several rules for the same network.

To list the current rules use:

.. code-block:: console

 # proxmox-backup-manager traffic-control list
 ┌───────┬─────────────┬─────────────┬─────────────────────────┬────────────...─┐
 │ name  │     rate-in │    rate-out │ network                 │ timeframe  ... │
 ╞═══════╪═════════════╪═════════════╪═════════════════════════╪════════════...═╡
 │ rule0 │      100 MB │      100 MB │ ["0.0.0.0/0"]           │ ["mon..fri ... │
 ├───────┼─────────────┼─────────────┼─────────────────────────┼────────────...─┤
 │ rule1 │       20 GB │       20 GB │ ["192.168.2.0/24", ...] │            ... │
 └───────┴─────────────┴─────────────┴─────────────────────────┴────────────...─┘

Rules can also be removed:

.. code-block:: console

 # proxmox-backup-manager traffic-control remove rule1


To show the state (current data rate) of all configured rules use:

.. code-block:: console

  # proxmox-backup-manager traffic-control traffic
  ┌───────┬─────────────┬──────────────┐
  │ name  │ cur-rate-in │ cur-rate-out │
  ╞═══════╪═════════════╪══════════════╡
  │ rule0 │         0 B │          0 B │
  ├───────┼─────────────┼──────────────┤
  │ rule1 │   1.161 GiB │   19.146 KiB │
  └───────┴─────────────┴──────────────┘
