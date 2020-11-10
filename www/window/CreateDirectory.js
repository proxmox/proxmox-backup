Ext.define('PBS.window.CreateDirectory', {
    extend: 'Proxmox.window.Edit',
    xtype: 'pbsCreateDirectory',

    subject: Proxmox.Utils.directoryText,
    showProgress: true,
    isCreate: true,
    url: '/nodes/localhost/disks/directory',
    method: 'POST',

    onlineHelp: 'storage-disk-management',

    items: [
	{
	    xtype: 'pmxDiskSelector',
	    name: 'disk',
	    valueField: 'name',
	    typeProperty: 'usage-type',
	    nodename: 'localhost',
	    diskType: 'unused',
	    fieldLabel: gettext('Disk'),
	    allowBlank: false,
	},
	{
	    xtype: 'proxmoxKVComboBox',
	    comboItems: [
		['ext4', 'ext4'],
		['xfs', 'xfs'],
	    ],
	    fieldLabel: gettext('Filesystem'),
	    name: 'filesystem',
	    value: '',
	    allowBlank: false,
	},
	{
	    xtype: 'proxmoxtextfield',
	    name: 'name',
	    fieldLabel: gettext('Name'),
	    allowBlank: false,
	},
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'add-datastore',
	    fieldLabel: gettext('Add as Datastore'),
	    value: '1',
	},
    ],
});

