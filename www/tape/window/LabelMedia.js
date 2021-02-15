Ext.define('PBS.TapeManagement.LabelMediaWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsLabelMediaWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    isCreate: true,
    isAdd: true,
    title: gettext('Label Media'),
    submitText: gettext('OK'),

    url: '/api2/extjs/tape/drive/',

    cbindData: function(config) {
	let me = this;
	return {
	    driveid: config.driveid,
	};
    },

    method: 'POST',

    showProgress: true,

    submitUrl: function(url, values) {
	let driveid = encodeURIComponent(values.drive);
	delete values.drive;
	return `${url}/${driveid}/label-media`;
    },

    items: [
	{
	    xtype: 'displayfield',
	    cls: 'pmx-hint',
	    value: gettext('Make sure that the correct tape is inserted the selected drive and type in the label written on the tape.'),
	},
	{
	    xtype: 'pmxDisplayEditField',
	    fieldLabel: gettext('Drive'),
	    name: 'drive',
	    editConfig: {
		xtype: 'pbsDriveSelector',
	    },
	    cbind: {
		value: '{driveid}',
		editable: '{!driveid}',
	    },
	},
	{
	    fieldLabel: gettext('Label'),
	    name: 'label-text',
	    xtype: 'proxmoxtextfield',
	    allowBlank: false,
	},
	{
	    xtype: 'pbsMediaPoolSelector',
	    fieldLabel: gettext('Media Pool'),
	    name: 'pool',
	    allowBlank: true,
	    skipEmptyText: true,
	},
    ],
});

