Ext.define('PBS.TapeManagement.EraseWindow', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],


    changer: undefined,
    label: undefined,

    cbindData: function(config) {
	let me = this;
	return {};
    },

    title: gettext('Erase'),
    url: `/api2/extjs/tape/drive`,
    showProgress: true,
    submitUrl: function(url, values) {
	let drive = values.drive;
	delete values.drive;
	return `${url}/${drive}/erase-media`;
    },

    method: 'POST',
    items: [
	{
	    xtype: 'displayfield',
	    cls: 'pmx-hint',
	    value: gettext('Make sure to insert the tape into the selected drive.'),
	    cbind: {
		hidden: '{changer}',
	    },
	},
	{
	    xtype: 'displayfield',
	    name: 'label-text',
	    submitValue: true,
	    fieldLabel: gettext('Media'),
	    cbind: {
		value: '{label}',
	    },
	},
	{
	    xtype: 'pbsDriveSelector',
	    fieldLabel: gettext('Drive'),
	    name: 'drive',
	    cbind: {
		changer: '{changer}',
	    },
	},
    ],
});
