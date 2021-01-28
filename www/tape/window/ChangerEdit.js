Ext.define('PBS.TapeManagement.ChangerEditWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsChangerEditWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    isCreate: true,
    isAdd: true,
    subject: gettext('Changer'),
    cbindData: function(initialConfig) {
	let me = this;

	let changerid = initialConfig.changerid;
	let baseurl = '/api2/extjs/config/changer';

	me.isCreate = !changerid;
	me.url = changerid ? `${baseurl}/${encodeURIComponent(changerid)}` : baseurl;
	me.method = changerid ? 'PUT' : 'POST';

	return { };
    },

    items: [
	{
	    fieldLabel: gettext('Name'),
	    name: 'name',
	    xtype: 'pmxDisplayEditField',
	    renderer: Ext.htmlEncode,
	    allowBlank: false,
	    cbind: {
		editable: '{isCreate}',
	    },
	},
	{
	    fieldLabel: gettext('Path'),
	    xtype: 'pbsTapeDevicePathSelector',
	    type: 'changers',
	    name: 'path',
	    allowBlank: false,
	},
	{
	    fieldLabel: gettext('Import-Export Slots'),
	    xtype: 'proxmoxtextfield',
	    name: 'export-slots',
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
    ],
});

