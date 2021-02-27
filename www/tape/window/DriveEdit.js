Ext.define('PBS.TapeManagement.DriveEditWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsDriveEditWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    isCreate: true,
    isAdd: true,
    subject: gettext('Drive'),
    cbindData: function(initialConfig) {
	let me = this;

	let driveid = initialConfig.driveid;
	let baseurl = '/api2/extjs/config/drive';

	me.isCreate = !driveid;
	me.url = driveid ? `${baseurl}/${encodeURIComponent(driveid)}` : baseurl;
	me.method = driveid ? 'PUT' : 'POST';

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
	    fieldLabel: gettext('Changer'),
	    xtype: 'pbsChangerSelector',
	    name: 'changer',
	    skipEmptyText: true,
	    allowBlank: true,
	    autoSelect: false,
	    emptyText: gettext('No Changer'),
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	    listeners: {
		change: function(field, value) {
		    let disableSlotField = !value || value === '';
		    console.log(value);
		    field
			.up('window')
			.down('field[name=changer-drivenum]')
			.setDisabled(disableSlotField);
		},
	    },
	},
	{
	    fieldLabel: gettext('Drive Number'),
	    xtype: 'proxmoxintegerfield',
	    name: 'changer-drivenum',
	    disabled: true,
	    allowBlank: true,
	    emptyText: '0',
	    minValue: 0,
	    maxValue: 8,
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
	{
	    fieldLabel: gettext('Path'),
	    xtype: 'pbsTapeDevicePathSelector',
	    type: 'drives',
	    name: 'path',
	    allowBlank: false,
	},
    ],
});

