Ext.define('PBS.TapeManagement.TapeBackupWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'pbsTapeBackupWindow',

    width: 400,
    subject: gettext('Backup'),
    url: '/api2/extjs/tape/backup',
    method: 'POST',
    showTaskViewer: true,
    isCreate: true,

    defaults: {
	labelWidth: 150,
    },

    items: [
	{
	    xtype: 'pbsDataStoreSelector',
	    fieldLabel: gettext('Datastore'),
	    name: 'store',
	},
	{
	    xtype: 'pbsMediaPoolSelector',
	    fieldLabel: gettext('Media Pool'),
	    name: 'pool',
	},
	{
	    xtype: 'pbsDriveSelector',
	    fieldLabel: gettext('Drive'),
	    name: 'drive',
	},
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'force-media-set',
	    fieldLabel: gettext('Force new Media Set'),
	},
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'export-media-set',
	    fieldLabel: gettext('Export Media Set'),
	    listeners: {
		change: function(cb, value) {
		    let me = this;
		    let eject = me.up('window').down('proxmoxcheckbox[name=eject-media]');
		    if (value) {
			eject.setValue(false);
		    }
		    eject.setDisabled(!!value);
		},
	    },
	},
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'eject-media',
	    fieldLabel: gettext('Eject Media'),
	},
	{
	    xtype: 'pbsUserSelector',
	    name: 'notify-user',
	    fieldLabel: gettext('Notify User'),
	    emptyText: 'root@pam',
	    value: null,
	    allowBlank: true,
	    skipEmptyText: true,
	    renderer: Ext.String.htmlEncode,
	},
    ],
});
