This daemon exposes the whole Proxmox Backup Server API on TCP port 8007 using
HTTPS. It runs as user ``backup`` and has very limited permissions. Operations
requiring more permissions are forwarded to the local ``proxmox-backup``
service.
