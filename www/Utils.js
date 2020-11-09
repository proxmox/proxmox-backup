Ext.ns('PBS');

console.log("Starting Backup Server GUI");

Ext.define('PBS.Utils', {
    singleton: true,

    updateLoginData: function(data) {
	Proxmox.Utils.setAuthData(data);
    },

    dataStorePrefix: 'DataStore-',

    cryptmap: [
	'none',
	'mixed',
	'sign-only',
	'encrypt',
    ],

    cryptText: [
	Proxmox.Utils.noText,
	gettext('Mixed'),
	gettext('Signed'),
	gettext('Encrypted'),
    ],

    cryptIconCls: [
	'',
	'',
	'lock faded',
	'lock good',
    ],

    calculateCryptMode: function(data) {
	let mixed = data.mixed;
	let encrypted = data.encrypt;
	let signed = data['sign-only'];
	let files = data.count;
	if (mixed > 0) {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	} else if (files === encrypted && encrypted > 0) {
	    return PBS.Utils.cryptmap.indexOf('encrypt');
	} else if (files === signed && signed > 0) {
	    return PBS.Utils.cryptmap.indexOf('sign-only');
	} else if ((signed+encrypted) === 0) {
	    return PBS.Utils.cryptmap.indexOf('none');
	} else {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	}
    },

    getDataStoreFromPath: function(path) {
	return path.slice(PBS.Utils.dataStorePrefix.length);
    },

    isDataStorePath: function(path) {
	return path.indexOf(PBS.Utils.dataStorePrefix) === 0;
    },

    parsePropertyString: function(value, defaultKey) {
	var res = {},
	    error;

	if (typeof value !== 'string' || value === '') {
	    return res;
	}

	Ext.Array.each(value.split(','), function(p) {
	    var kv = p.split('=', 2);
	    if (Ext.isDefined(kv[1])) {
		res[kv[0]] = kv[1];
	    } else if (Ext.isDefined(defaultKey)) {
		if (Ext.isDefined(res[defaultKey])) {
		    error = 'defaultKey may be only defined once in propertyString';
		    return false; // break
		}
		res[defaultKey] = kv[0];
	    } else {
		error = 'invalid propertyString, not a key=value pair and no defaultKey defined';
		return false; // break
	    }
	    return true;
	});

	if (error !== undefined) {
	    console.error(error);
	    return null;
	}

	return res;
    },

    printPropertyString: function(data, defaultKey) {
	var stringparts = [],
	    gotDefaultKeyVal = false,
	    defaultKeyVal;

	Ext.Object.each(data, function(key, value) {
	    if (defaultKey !== undefined && key === defaultKey) {
		gotDefaultKeyVal = true;
		defaultKeyVal = value;
	    } else if (value !== '' && value !== undefined) {
		stringparts.push(key + '=' + value);
	    }
	});

	stringparts = stringparts.sort();
	if (gotDefaultKeyVal) {
	    stringparts.unshift(defaultKeyVal);
	}

	return stringparts.join(',');
    },

    // helper for deleting field which are set to there default values
    delete_if_default: function(values, fieldname, default_val, create) {
	if (values[fieldname] === '' || values[fieldname] === default_val) {
	    if (!create) {
		if (values.delete) {
		    if (Ext.isArray(values.delete)) {
			values.delete.push(fieldname);
		    } else {
			values.delete += ',' + fieldname;
		    }
		} else {
		    values.delete = [fieldname];
		}
	    }

	    delete values[fieldname];
	}
    },


    render_datetime_utc: function(datetime) {
	let pad = (number) => number < 10 ? '0' + number : number;
	return datetime.getUTCFullYear() +
	    '-' + pad(datetime.getUTCMonth() + 1) +
	    '-' + pad(datetime.getUTCDate()) +
	    'T' + pad(datetime.getUTCHours()) +
	    ':' + pad(datetime.getUTCMinutes()) +
	    ':' + pad(datetime.getUTCSeconds()) +
	    'Z';
    },

    render_datastore_worker_id: function(id, what) {
	const res = id.match(/^(\S+?)_(\S+?)_(\S+?)(_(.+))?$/);
	if (res) {
	    let datastore = res[1], backupGroup = `${res[2]}/${res[3]}`;
	    if (res[4] !== undefined) {
		let datetime = Ext.Date.parse(parseInt(res[5], 16), 'U');
		let utctime = PBS.Utils.render_datetime_utc(datetime);
		return `Datastore ${datastore} ${what} ${backupGroup}/${utctime}`;
	    } else {
		return `Datastore ${datastore} ${what} ${backupGroup}`;
	    }
	}
	return `Datastore ${what} ${id}`;
    },

    extractTokenUser: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[1];
    },

    extractTokenName: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[2];
    },

    render_estimate: function(value) {
	if (!value) {
	    return gettext('Not enough data');
	}

	let now = new Date();
	let estimate = new Date(value*1000);

	let timespan = (estimate - now)/1000;

	if (Number(estimate) <= Number(now) || isNaN(timespan)) {
	    return gettext('Never');
	}

	let duration = Proxmox.Utils.format_duration_human(timespan);
	return Ext.String.format(gettext("in {0}"), duration);
    },

    constructor: function() {
	var me = this;

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    backup: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Backup')),
	    dircreate: [gettext('Directory Storage'), gettext('Create')],
	    dirremove: [gettext('Directory'), gettext('Remove')],
	    garbage_collection: ['Datastore', gettext('Garbage collect')],
	    logrotate: [null, gettext('Log Rotation')],
	    prune: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Prune')),
	    reader: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Read objects')),
	    sync: ['Datastore', gettext('Remote Sync')],
	    syncjob: [gettext('Sync Job'), gettext('Remote Sync')],
	    verify: ['Datastore', gettext('Verification')],
	    verify_group: ['Group', gettext('Verification')],
	    verify_snapshot: ['Snapshot', gettext('Verification')],
	    verificationjob: [gettext('Verify Job'), gettext('Scheduled Verification')],
	    zfscreate: [gettext('ZFS Storage'), gettext('Create')],
	});
    },
});
