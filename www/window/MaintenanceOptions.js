Ext.define('PBS.form.maintenanceType', {
    extend: 'Proxmox.form.KVComboBox',
    alias: 'widget.pbsMaintenanceType',

    comboItems: [
	['__default__', gettext('None')],
	['read-only', gettext('Read only')],
	['offline', gettext('Offline')],
    ],
});

Ext.define('PBS.window.MaintenanceOptions', {
    extend: 'Proxmox.window.Edit',
    xtype: 'pbsMaintenanceOptionEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    subject: gettext('Maintenance mode'),

    width: 450,
    fieldDefaults: {
	labelWidth: 120,
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    if (values.delete === 'maintenance-type') {
		values.delete = 'maintenance-mode';
	    } else if (values['maintenance-type']) {
		const escaped_message = (values['maintenance-msg'] ?? '')
		    .replaceAll('\\', '')
		    .replaceAll('"', '\\"');
		const maybe_message =
		    values['maintenance-msg'] ? `,message="${escaped_message}"` : '';
		values['maintenance-mode'] = `type=${values['maintenance-type']}${maybe_message}`;
	    }
	    delete values['maintenance-type'];
	    delete values['maintenance-msg'];
	    return values;
	},
	items: [
	    {
		xtype: 'pbsMaintenanceType',
		name: 'maintenance-type',
		fieldLabel: gettext('Maintenance Type'),
		value: '__default__',
		deleteEmpty: true,
	    },
	    {
		xtype: 'proxmoxtextfield',
		name: 'maintenance-msg',
		fieldLabel: gettext('Description'),
		// FIXME: disable if maintenance type is none
	    },
	],
    },
    setValues: function(values) {
	let me = this;

	let options = {
	    'maintenance-type': '__default__',
	    'maintenance-msg': '',
	};
	if (values['maintenance-mode']) {
	    let [type, message] = values['maintenance-mode'].split(/,(.+)/);
	    type = type.split("=").pop();
	    message = message ? message.split("=")[1]
		.replace(/^"(.*)"$/, '$1')
		.replaceAll('\\"', '"') : '';
	    options = {
		'maintenance-type': type,
		'maintenance-msg': message,
	    };
	}

	me.callParent([options]);
    },
});
