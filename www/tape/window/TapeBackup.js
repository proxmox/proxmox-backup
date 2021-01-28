Ext.define('PBS.TapeManagement.TapeBackupWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'pbsTapeBackupWindow',

    subject: gettext('Backup'),
    url: '/api2/extjs/tape/backup',
    method: 'POST',
    showTaskViewer: true,
    isCreate: true,

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
    ],
});
