Ext.define('PBS.TapeManagement.TapeRestoreWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'pbsTapeRestoreWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    width: 400,
    title: gettext('Restore Media Set'),
    url: '/api2/extjs/tape/restore',
    method: 'POST',
    showTaskViewer: true,
    isCreate: true,

    defaults: {
	labelWidth: 120,
    },

    items: [
	{
	    xtype: 'displayfield',
	    fieldLabel: gettext('Media Set'),
	    cbind: {
		value: '{mediaset}',
	    },
	},
	{
	    xtype: 'displayfield',
	    fieldLabel: gettext('Media Set UUID'),
	    name: 'media-set',
	    submitValue: true,
	    cbind: {
		value: '{uuid}',
	    },
	},
	{
	    xtype: 'pbsDataStoreSelector',
	    fieldLabel: gettext('Datastore'),
	    name: 'store',
	},
	{
	    xtype: 'pbsDriveSelector',
	    fieldLabel: gettext('Drive'),
	    name: 'drive',
	},
	{
	    xtype: 'pbsUserSelector',
	    name: 'notify-user',
	    fieldLabel: gettext('Notify User'),
	    emptyText: gettext('Current User'),
	    value: null,
	    allowBlank: true,
	    skipEmptyText: true,
	    renderer: Ext.String.htmlEncode,
	},
    ],
});
